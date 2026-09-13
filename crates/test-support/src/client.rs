//! 仅回环的测试 HTTP 客户端（"真实 HTTP 客户端"的最小实现，供 fixture 驱动用）。
//!
//! 隔离机制（REQ-008 / AC-014："测试进程不得产生真实外网调用"）：
//! - 目标 URL 的主机**必须是 IP 字面量**（不做 DNS 解析）且必须 `is_loopback()`；
//!   主机名（`example.com`）、公网 IP、`https://` 一律在建立连接**之前**拒绝；
//! - 因此测试代码在物理上无法借此发起真实外网调用；负例断言见
//!   `fixture_harness.rs::guarded_client_refuses_non_loopback_targets`。
//!
//! T12/T14 的真实适配器（reqwest）不需要复用本客户端；本客户端用于在 T05
//! 证明 harness 的字节级行为（成功/延迟/断连/429/5xx/畸形 JSON/超时）。

use std::fmt;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::Duration;

use serde_json::Value;

/// 客户端错误（测试据此断言"断连/超时/截断"确实发生，而不是假成功）。
#[derive(Debug)]
pub enum ClientError {
    /// 主机不是 IP 字面量（拒绝 DNS，避免解析到外网）。
    NotIpLiteral { host: String },
    /// IP 不是回环地址。
    NotLoopback { host: String },
    /// 非 http scheme（本机 fixture 无 TLS）。
    UnsupportedScheme { scheme: String },
    /// URL 无法解析。
    MalformedUrl(String),
    /// 连接失败。
    Connect { addr: SocketAddr, message: String },
    /// 读/写超时（fixture 的 timeout 场景）。
    Timeout,
    /// 未收到任何响应字节前连接被关闭（fixture 的 disconnect / reset 场景）。
    ConnectionClosed,
    /// 响应体短于 content-length（fixture 的 halfClose 截断场景）。
    TruncatedBody { expected: usize, actual: usize },
    /// 其它 IO 错误（如 RST → ConnectionReset）。
    Io(std::io::Error),
    /// 响应状态行/头格式非法。
    MalformedResponse(String),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::NotIpLiteral { host } => {
                write!(f, "拒绝非 IP 字面量主机（测试客户端不做 DNS 解析）：{host}")
            }
            ClientError::NotLoopback { host } => {
                write!(f, "拒绝非回环地址（测试禁止真实外网调用）：{host}")
            }
            ClientError::UnsupportedScheme { scheme } => {
                write!(f, "测试客户端只支持 http://（本机 fixture）：{scheme}://")
            }
            ClientError::MalformedUrl(url) => write!(f, "URL 无法解析：{url}"),
            ClientError::Connect { addr, message } => write!(f, "连接 {addr} 失败：{message}"),
            ClientError::Timeout => write!(f, "等待响应超时"),
            ClientError::ConnectionClosed => write!(f, "连接在响应前被关闭（无任何字节）"),
            ClientError::TruncatedBody { expected, actual } => {
                write!(
                    f,
                    "响应体被截断：content-length={expected}，实际 {actual} 字节"
                )
            }
            ClientError::Io(error) => write!(f, "IO 错误：{error}"),
            ClientError::MalformedResponse(detail) => write!(f, "响应格式非法：{detail}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<std::io::Error> for ClientError {
    fn from(error: std::io::Error) -> Self {
        if matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ) {
            ClientError::Timeout
        } else {
            ClientError::Io(error)
        }
    }
}

/// 解析后的响应。
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// 大小写不敏感取头值。
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// 响应体文本。
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// 解析响应体 JSON（畸形 JSON 场景应由调用方断言 `is_err`）。
    pub fn json(&self) -> Result<Value, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }
}

/// 仅回环的 HTTP 客户端。
#[derive(Debug, Clone)]
pub struct LocalHttpClient {
    read_timeout: Duration,
    connect_timeout: Duration,
}

impl Default for LocalHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalHttpClient {
    /// 默认读超时 1 秒（fixture 的 timeout 场景需明显更长才会触发）。
    pub fn new() -> Self {
        Self {
            read_timeout: Duration::from_secs(1),
            connect_timeout: Duration::from_millis(500),
        }
    }

    /// 覆盖读超时（如需要观察延迟场景）。
    pub fn with_read_timeout(read_timeout: Duration) -> Self {
        Self {
            read_timeout,
            ..Self::new()
        }
    }

    /// GET。
    pub fn get(&self, url: &str) -> Result<HttpResponse, ClientError> {
        self.request("GET", url, &[], None)
    }

    /// JSON POST（默认 `content-type: application/json`）。
    pub fn post_json(&self, url: &str, body: &Value) -> Result<HttpResponse, ClientError> {
        let bytes = serde_json::to_vec(body).expect("序列化 JSON");
        self.request(
            "POST",
            url,
            &[("content-type", "application/json")],
            Some(&bytes),
        )
    }

    /// 原始字节 POST。
    pub fn post_bytes(
        &self,
        url: &str,
        content_type: Option<&str>,
        body: &[u8],
    ) -> Result<HttpResponse, ClientError> {
        let mut headers: Vec<(&str, &str)> = Vec::new();
        if let Some(content_type) = content_type {
            headers.push(("content-type", content_type));
        }
        self.request("POST", url, &headers, Some(body))
    }

    /// 通用请求。
    ///
    /// **准入检查先于任何 socket 操作**：非 `http://`、非 IP 字面量主机、
    /// 非回环地址直接返回错误（见 [`ClientError`]），不做 DNS 解析。
    pub fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<HttpResponse, ClientError> {
        let (addr, host_header, path) = parse_loopback_url(url)?;
        let mut stream =
            TcpStream::connect_timeout(&addr, self.connect_timeout).map_err(|error| {
                ClientError::Connect {
                    addr,
                    message: error.to_string(),
                }
            })?;
        let _ = stream.set_read_timeout(Some(self.read_timeout));
        let _ = stream.set_write_timeout(Some(self.read_timeout));

        let mut request = Vec::new();
        request.extend_from_slice(format!("{method} {path} HTTP/1.1\r\n").as_bytes());
        request.extend_from_slice(format!("host: {host_header}\r\n").as_bytes());
        let mut has_length = false;
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("content-length") {
                has_length = true;
            }
            request.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        let body = body.unwrap_or(&[]);
        if !body.is_empty() && !has_length {
            request.extend_from_slice(format!("content-length: {}\r\n", body.len()).as_bytes());
        }
        request.extend_from_slice(b"connection: close\r\n\r\n");
        request.extend_from_slice(body);
        stream.write_all(&request)?;
        stream.flush()?;

        read_response(&mut stream)
    }
}

/// URL → `(socket 地址, host 头, 路径)`；非回环/非 IP 字面量在这里被拒绝。
fn parse_loopback_url(url: &str) -> Result<(SocketAddr, String, String), ClientError> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| ClientError::MalformedUrl(url.to_owned()))?;
    if scheme != "http" {
        return Err(ClientError::UnsupportedScheme {
            scheme: scheme.to_owned(),
        });
    }
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_owned()),
    };
    let authority = authority.split('@').next_back().unwrap_or(authority);
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        // IPv6 字面量：[::1]:port
        let (host, tail) = rest
            .split_once(']')
            .ok_or_else(|| ClientError::MalformedUrl(url.to_owned()))?;
        let port = match tail.strip_prefix(':') {
            Some(port) => port
                .parse::<u16>()
                .map_err(|_| ClientError::MalformedUrl(url.to_owned()))?,
            None => 80,
        };
        (host.to_owned(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (
                host.to_owned(),
                port.parse::<u16>()
                    .map_err(|_| ClientError::MalformedUrl(url.to_owned()))?,
            ),
            None => (authority.to_owned(), 80),
        }
    };

    let ip: IpAddr = match host.parse() {
        Ok(ip) => ip,
        Err(_) => return Err(ClientError::NotIpLiteral { host }),
    };
    if !ip.is_loopback() {
        return Err(ClientError::NotLoopback { host });
    }
    let host_header = match ip {
        IpAddr::V4(v4) => format!("{v4}:{port}"),
        IpAddr::V6(v6) => format!("[{v6}]:{port}"),
    };
    Ok((SocketAddr::new(ip, port), host_header, path))
}

/// 读完整响应（content-length 或读到 EOF），并检测截断。
fn read_response(stream: &mut TcpStream) -> Result<HttpResponse, ClientError> {
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => raw.extend_from_slice(&buffer[..read]),
            // 超时（无字节）与连接中断（RST）都以错误返回：不把"没读到完整响应"
            // 误判成成功。
            Err(error) => return Err(ClientError::from(error)),
        }
    }
    if raw.is_empty() {
        return Err(ClientError::ConnectionClosed);
    }

    let header_end = find_header_end(&raw)
        .ok_or_else(|| ClientError::MalformedResponse("未找到响应头结束标记".to_owned()))?;
    let head = String::from_utf8_lossy(&raw[..header_end]).into_owned();
    let body = &raw[header_end + 4..];
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| ClientError::MalformedResponse(format!("非法状态行：{status_line:?}")))?;
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| ClientError::MalformedResponse(format!("非法响应头：{line:?}")))?;
        headers.push((name.trim().to_owned(), value.trim().to_owned()));
    }

    if let Some((_, expected)) = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        && let Ok(expected) = expected.parse::<usize>()
        && body.len() < expected
    {
        return Err(ClientError::TruncatedBody {
            expected,
            actual: body.len(),
        });
    }

    Ok(HttpResponse {
        status,
        headers,
        body: body.to_vec(),
    })
}

fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|window| window == b"\r\n\r\n")
}

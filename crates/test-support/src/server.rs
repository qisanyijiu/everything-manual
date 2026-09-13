//! 本机 fixture HTTP 服务器（std 实现，无异步运行时依赖）。
//!
//! 约束与设计（REQ-008 / AC-014）：
//! - **只绑定 `127.0.0.1:0`**（随机端口）：测试不可能把 fixture 暴露到非回环；
//! - 逐请求按 [`ResolvedScenario`] 的脚本执行：成功 / 延迟 / 断连（FIN）/ RST /
//!   半关闭截断 / 429+Retry-After / 5xx / 畸形 JSON / 超时；
//! - **每次请求先记录再执行**：即使断连或超时也能断言"调用了几次、发了什么"；
//! - **缺脚本必须失败**：没有匹配路由或步骤耗尽（且未声明 `repeatLast`）时返回
//!   `501` 并记入 script problem，绝不返回通用成功；
//! - 每个响应带 `connection: close`，每请求一条连接，脚本游标可预期。

use std::io::{Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::record::{HeaderView, RecordedRequest, RecordingOutcome, redact_header};
use crate::scenario::{ResolvedResponse, ResolvedScenario, ResolvedStep, Scenario};

/// 请求头/请求行上限（防御手滑的测试客户端；fixture 只服务小请求）。
const MAX_HEAD_BYTES: usize = 64 * 1024;
/// 请求体上限（样例资产远小于此；防止误发大文件吃内存）。
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// 脚本问题（缺路由 / 步骤耗尽）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptProblemKind {
    /// 没有任何路由匹配该方法与路径。
    NoRoute,
    /// 路由存在但步骤耗尽且未声明 `repeatLast`。
    ScriptExhausted { route_index: usize },
}

/// 一条脚本问题记录（用于断言"缺脚本必须失败"）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptProblem {
    pub method: String,
    pub path: String,
    pub kind: ScriptProblemKind,
}

impl ScriptProblem {
    pub fn describe(&self) -> String {
        match self.kind {
            ScriptProblemKind::NoRoute => format!(
                "fixture 无脚本：{} {}（返回 501，未返回通用成功）",
                self.method, self.path
            ),
            ScriptProblemKind::ScriptExhausted { route_index } => format!(
                "fixture 脚本步骤耗尽：{} {}（路由 #{}；如需重复请声明 repeatLast）",
                self.method, self.path, route_index
            ),
        }
    }
}

/// 本机 fixture 服务器句柄；`Drop` 时停止监听并唤醒挂起请求。
pub struct FixtureServer {
    inner: Arc<Inner>,
    addr: SocketAddr,
    accept_thread: Option<JoinHandle<()>>,
}

struct Inner {
    state: Mutex<State>,
    /// 用于可中断等待（delay / timeout 场景可被 Drop 提前唤醒）。
    signal: Condvar,
    shutdown: AtomicBool,
}

struct State {
    scenario: ResolvedScenario,
    next_step: Vec<usize>,
    recordings: Vec<RecordedRequest>,
    problems: Vec<ScriptProblem>,
    sequence: usize,
}

impl FixtureServer {
    /// 启动场景（panic 信息包含场景解析错误，例如响应体文件缺失）。
    pub fn start(scenario: Scenario) -> Self {
        Self::start_resolved(scenario.resolve())
    }

    /// 启动 `tests/fixtures/scenarios/<name>`。
    pub fn from_scenario_file(name: &str) -> Self {
        Self::start(Scenario::load_scenario(name))
    }

    /// 启动已解析场景（测试内直接构造 `ResolvedStep` 时使用）。
    pub fn start_resolved(scenario: ResolvedScenario) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("绑定 127.0.0.1:0");
        let addr = listener.local_addr().expect("读取监听地址");
        assert!(
            addr.ip().is_loopback(),
            "fixture 只允许回环监听，实际绑定 {addr}"
        );

        let inner = Arc::new(Inner {
            state: Mutex::new(State {
                next_step: vec![0; scenario.routes.len()],
                scenario,
                recordings: Vec::new(),
                problems: Vec::new(),
                sequence: 0,
            }),
            signal: Condvar::new(),
            shutdown: AtomicBool::new(false),
        });

        let accept_inner = Arc::clone(&inner);
        let accept_thread = std::thread::Builder::new()
            .name("fixture-accept".to_owned())
            .spawn(move || accept_loop(listener, accept_inner))
            .expect("启动 fixture 监听线程");

        Self {
            inner,
            addr,
            accept_thread: Some(accept_thread),
        }
    }

    /// `127.0.0.1:<随机端口>`。
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// `http://127.0.0.1:<port>`。
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// `base_url` + 路径（路径需以 `/` 开头）。
    pub fn url(&self, path: &str) -> String {
        assert!(path.starts_with('/'), "路径需以 / 开头：{path}");
        format!("http://{}{}", self.addr, path)
    }

    /// 全部记录（按到达顺序克隆）。
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.inner
            .state
            .lock()
            .expect("fixture 状态锁")
            .recordings
            .clone()
    }

    /// 方法 + 路径完全相等的记录；`method` 传 `*` 表示不限方法。
    pub fn requests_matching(&self, method: &str, path: &str) -> Vec<RecordedRequest> {
        self.requests()
            .into_iter()
            .filter(|request| {
                (method == "*" || request.method.eq_ignore_ascii_case(method))
                    && request.path == path
            })
            .collect()
    }

    /// 调用次数（方法大小写不敏感、路径完全相等）。"付费 POST 只发 1 次"用这个断言。
    pub fn call_count(&self, method: &str, path: &str) -> usize {
        self.requests_matching(method, path).len()
    }

    /// 总请求数（用于断言"没有任何外呼发生"）。
    pub fn request_total(&self) -> usize {
        self.inner
            .state
            .lock()
            .expect("fixture 状态锁")
            .recordings
            .len()
    }

    /// 断言某路由恰好被调用一次（失败信息列出全部已记录请求）。
    pub fn assert_called_once(&self, method: &str, path: &str) {
        self.assert_called_times(method, path, 1);
    }

    /// 断言某路由的调用次数。
    pub fn assert_called_times(&self, method: &str, path: &str, expected: usize) {
        let matching = self.requests_matching(method, path);
        assert_eq!(
            matching.len(),
            expected,
            "期望 {method} {path} 被调用 {expected} 次，实际 {} 次。全部记录：{}",
            matching.len(),
            self.recorded_summary().join("；")
        );
    }

    /// 全部记录的摘要行（断言失败与 implementation 记录用）。
    pub fn recorded_summary(&self) -> Vec<String> {
        self.requests()
            .iter()
            .map(RecordedRequest::summary)
            .collect()
    }

    /// 脚本问题（缺路由 / 步骤耗尽）。
    pub fn script_problems(&self) -> Vec<ScriptProblem> {
        self.inner
            .state
            .lock()
            .expect("fixture 状态锁")
            .problems
            .clone()
    }

    /// 断言 fixture 从未因"未按脚本命中"返回失败响应。
    pub fn assert_no_script_problems(&self) {
        let problems = self.script_problems();
        assert!(
            problems.is_empty(),
            "fixture 出现脚本问题：{}",
            problems
                .iter()
                .map(ScriptProblem::describe)
                .collect::<Vec<_>>()
                .join("；")
        );
    }

    /// 主动停止（等价于 Drop）：停止接收新连接并唤醒挂起请求。
    pub fn shutdown(&self) {
        if !self.inner.shutdown.swap(true, Ordering::SeqCst) {
            self.inner.signal.notify_all();
            // 自连接唤醒阻塞在 accept 的监听线程。
            let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(200));
        }
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

fn accept_loop(listener: TcpListener, inner: Arc<Inner>) {
    for stream in listener.incoming() {
        if inner.shutdown.load(Ordering::SeqCst) {
            break;
        }
        match stream {
            Ok(stream) => {
                let handler_inner = Arc::clone(&inner);
                let _ = std::thread::Builder::new()
                    .name("fixture-conn".to_owned())
                    .spawn(move || {
                        let _ = handle_connection(stream, handler_inner);
                    });
            }
            Err(_) => {
                if inner.shutdown.load(Ordering::SeqCst) {
                    break;
                }
            }
        }
    }
}

/// 一个连接：读请求（含 100-continue 与 chunked）→ 记录 → 按脚本响应。
fn handle_connection(mut stream: TcpStream, inner: Arc<Inner>) -> std::io::Result<()> {
    let timeout = Some(Duration::from_millis(5_000));
    let _ = stream.set_read_timeout(timeout);
    let _ = stream.set_write_timeout(timeout);

    let Some(parsed) = read_request(&mut stream)? else {
        return Ok(());
    };

    let (step, outcome) = {
        let mut state = inner.state.lock().expect("fixture 状态锁");
        let (step, outcome) = state.pick_step(&parsed.method, &parsed.path);
        let sequence = state.sequence;
        state.sequence += 1;
        let headers = parsed
            .headers
            .iter()
            .map(|(name, value)| {
                let (value, redacted) = redact_header(name, value);
                HeaderView {
                    name: name.clone(),
                    value,
                    redacted,
                }
            })
            .collect();
        state.recordings.push(RecordedRequest {
            sequence,
            method: parsed.method.clone(),
            target: parsed.target.clone(),
            path: parsed.path.clone(),
            query: parsed.query.clone(),
            headers,
            body: parsed.body.clone(),
            outcome: outcome.clone(),
        });
        match &outcome {
            RecordingOutcome::NoRoute => state.problems.push(ScriptProblem {
                method: parsed.method.clone(),
                path: parsed.path.clone(),
                kind: ScriptProblemKind::NoRoute,
            }),
            RecordingOutcome::ScriptExhausted { route_index } => {
                state.problems.push(ScriptProblem {
                    method: parsed.method.clone(),
                    path: parsed.path.clone(),
                    kind: ScriptProblemKind::ScriptExhausted {
                        route_index: *route_index,
                    },
                });
            }
            RecordingOutcome::Scripted { .. } => {}
        }
        (step, outcome)
    };

    let Some(step) = step else {
        // 缺脚本：显式失败（501），不返回通用成功。
        let body = serde_json::json!({
            "error": "fixture script missing",
            "detail": match &outcome {
                RecordingOutcome::NoRoute => format!(
                    "no fixture route for {} {}",
                    parsed.method, parsed.path
                ),
                RecordingOutcome::ScriptExhausted { route_index } => format!(
                    "fixture route #{route_index} exhausted steps for {} {} (declare repeatLast to repeat)",
                    parsed.method, parsed.path
                ),
                RecordingOutcome::Scripted { .. } => "unreachable".to_owned(),
            },
        });
        let response = ResolvedResponse {
            status: 501,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body: serde_json::to_vec(&body).expect("序列化 501 响应"),
        };
        write_response(&mut stream, &response)?;
        return Ok(());
    };

    match step {
        ResolvedStep::Respond(response) => write_response(&mut stream, &response)?,
        ResolvedStep::Delay { delay_ms, response } => {
            inner.sleep_interruptible(Duration::from_millis(delay_ms));
            if !inner.shutdown.load(Ordering::SeqCst) {
                write_response(&mut stream, &response)?;
            }
        }
        ResolvedStep::Disconnect => {
            // 直接关闭连接：客户端读到 EOF 且没有任何响应字节。
        }
        ResolvedStep::Reset => {
            // SO_LINGER=0 + close → RST：客户端读到 ConnectionReset。
            set_linger_zero(&stream);
        }
        ResolvedStep::HalfClose {
            response,
            truncate_at,
        } => {
            write_response_head_and_prefix(&mut stream, &response, truncate_at)?;
            let _ = stream.shutdown(Shutdown::Write);
            // 半关闭后短暂保留读方向（真实半关闭语义），随后关闭。
            let mut sink = [0_u8; 1024];
            let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
            while let Ok(read) = stream.read(&mut sink) {
                if read == 0 {
                    break;
                }
            }
        }
        ResolvedStep::Timeout { hold_ms } => {
            inner.sleep_interruptible(Duration::from_millis(hold_ms));
            // 不写任何字节，直接关闭（客户端应已读超时）。
        }
    }
    Ok(())
}

impl State {
    /// 取"下一步"；返回应执行的步骤与记录结果。
    fn pick_step(&mut self, method: &str, path: &str) -> (Option<ResolvedStep>, RecordingOutcome) {
        let route_index = self
            .scenario
            .routes
            .iter()
            .position(|route| route.matches(method, path));
        let Some(route_index) = route_index else {
            return (None, RecordingOutcome::NoRoute);
        };
        let route = &self.scenario.routes[route_index];
        let cursor = self.next_step[route_index];
        if cursor < route.steps.len() {
            self.next_step[route_index] = cursor + 1;
            return (
                Some(route.steps[cursor].clone()),
                RecordingOutcome::Scripted {
                    route_index,
                    step_index: cursor,
                },
            );
        }
        if route.repeat_last && !route.steps.is_empty() {
            let step_index = route.steps.len() - 1;
            return (
                Some(route.steps[step_index].clone()),
                RecordingOutcome::Scripted {
                    route_index,
                    step_index,
                },
            );
        }
        (None, RecordingOutcome::ScriptExhausted { route_index })
    }
}

impl Inner {
    /// 可被 shutdown 唤醒的等待（用于 delay / timeout 场景，避免测试进程挂住）。
    fn sleep_interruptible(&self, duration: Duration) {
        let deadline = Instant::now() + duration;
        let mut guard = self.state.lock().expect("fixture 状态锁");
        while !self.shutdown.load(Ordering::SeqCst) {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let (next_guard, _) = self
                .signal
                .wait_timeout(guard, deadline - now)
                .expect("fixture 等待");
            guard = next_guard;
        }
    }
}

/// 解析后的请求。
struct ParsedRequest {
    method: String,
    /// 原始 request-target（含查询串）。
    target: String,
    path: String,
    query: Option<String>,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// 读取一个完整请求；连接在请求开始前关闭返回 `Ok(None)`。
fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<ParsedRequest>> {
    let head = match read_head(stream)? {
        Some(head) => head,
        None => return Ok(None),
    };
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    if method.is_empty() || target.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("fixture 收到非法请求行：{request_line:?}"),
        ));
    }
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_owned(), Some(query.to_owned())),
        None => (target.clone(), None),
    };

    let mut headers: Vec<(String, String)> = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("fixture 收到非法请求头：{line:?}"),
            ));
        };
        headers.push((name.trim().to_owned(), value.trim().to_owned()));
    }

    let header = |key: &str| {
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.clone())
    };

    if header("expect").is_some_and(|value| value.eq_ignore_ascii_case("100-continue")) {
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
        stream.flush()?;
    }

    let chunked = header("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"));
    let body = if chunked {
        read_chunked_body(stream)?
    } else {
        let length = header("content-length")
            .map(|value| {
                value.parse::<usize>().map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("fixture 收到非法 content-length：{value}"),
                    )
                })
            })
            .transpose()?
            .unwrap_or(0);
        if length > MAX_BODY_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("fixture 拒绝超过 {MAX_BODY_BYTES} 字节的请求体（{length} 字节）"),
            ));
        }
        let mut body = vec![0_u8; length];
        stream.read_exact(&mut body)?;
        body
    };

    Ok(Some(ParsedRequest {
        method,
        target,
        path,
        query,
        headers,
        body,
    }))
}

/// 读到 `\r\n\r\n` 为止（上限 [`MAX_HEAD_BYTES`]）；连接在首个字节前关闭返回 `Ok(None)`。
fn read_head(stream: &mut TcpStream) -> std::io::Result<Option<Vec<u8>>> {
    let mut head: Vec<u8> = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => {
                if head.is_empty() {
                    return Ok(None);
                }
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "fixture 在请求头完成前收到 EOF",
                ));
            }
            Ok(_) => {
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") {
                    head.truncate(head.len() - 4);
                    return Ok(Some(head));
                }
                if head.len() > MAX_HEAD_BYTES {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "fixture 请求头超过上限",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
    }
}

/// chunked 请求体（reqwest 流式 multipart 可能使用）。
fn read_chunked_body(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let line = read_line(stream)?;
        let size_text = line.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("fixture 收到非法 chunk 长度：{line:?}"),
            )
        })?;
        if size == 0 {
            // 末尾 trailers 直到空行。
            loop {
                let trailer = read_line(stream)?;
                if trailer.is_empty() {
                    break;
                }
            }
            return Ok(body);
        }
        if body.len() + size > MAX_BODY_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "fixture chunked 请求体超过上限",
            ));
        }
        let mut chunk = vec![0_u8; size];
        stream.read_exact(&mut chunk)?;
        body.extend_from_slice(&chunk);
        let mut crlf = [0_u8; 2];
        stream.read_exact(&mut crlf)?;
    }
}

fn read_line(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match stream.read(&mut byte)? {
            0 => break,
            _ => {
                if byte[0] == b'\n' {
                    break;
                }
                if byte[0] != b'\r' {
                    line.push(byte[0]);
                }
            }
        }
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

fn write_response(stream: &mut TcpStream, response: &ResolvedResponse) -> std::io::Result<()> {
    write_response_head_and_prefix(stream, response, response.body.len())
}

/// 写状态行 + 头（content-length 按**完整**响应体）+ 前 `prefix` 字节。
fn write_response_head_and_prefix(
    stream: &mut TcpStream,
    response: &ResolvedResponse,
    prefix: usize,
) -> std::io::Result<()> {
    let mut out = Vec::new();
    out.extend_from_slice(
        format!(
            "HTTP/1.1 {} {}\r\n",
            response.status,
            reason_phrase(response.status)
        )
        .as_bytes(),
    );
    let mut has_length = false;
    for (name, value) in &response.headers {
        if name.eq_ignore_ascii_case("content-length") {
            has_length = true;
        }
        out.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    if !has_length {
        out.extend_from_slice(format!("content-length: {}\r\n", response.body.len()).as_bytes());
    }
    out.extend_from_slice(b"connection: close\r\n\r\n");
    out.extend_from_slice(&response.body[..prefix.min(response.body.len())]);
    stream.write_all(&out)?;
    stream.flush()
}

/// `SO_LINGER=0`（close 时发 RST）。标准库的 `set_linger` 尚未稳定，用 libc 直调。
#[cfg(unix)]
fn set_linger_zero(stream: &TcpStream) {
    use std::os::fd::AsRawFd;
    let linger = libc::linger {
        l_onoff: 1,
        l_linger: 0,
    };
    // SAFETY：fd 来自存活的 TcpStream；指针与长度来自栈上 linger 结构。
    unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            std::ptr::from_ref(&linger).cast(),
            std::mem::size_of::<libc::linger>() as libc::socklen_t,
        );
    }
}

/// 非 Unix 平台退化为普通关闭（FIN）：`reset` 场景在扩展平台上语义略弱。
#[cfg(not(unix))]
fn set_linger_zero(_stream: &TcpStream) {}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Unknown",
    }
}

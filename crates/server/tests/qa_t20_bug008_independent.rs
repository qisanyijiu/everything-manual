//! QA 回合 26（BUG-008 复验）独立验收测试。
//!
//! 目的：验证 **下载阶段的失败消息** 不会把供应商签名 URL 带进
//! `job_stages.last_error` / `provider_attempts.last_error` / API 响应
//! （AC-010「任务元数据不得含临时云端 URL」的新增数据侧；contracts §1
//! 「不向用户输出……完整供应商签名 URL」）。
//!
//! 为什么单独写：`DownloadError::Transport` 的 detail 直接拼接 `reqwest::Error`，
//! 而 reqwest 的 Display 在请求错误后追加 ` for url (<完整 URL，含查询串>)`。
//! 生产里 CDN 超时/半途断连会走到这条分支；本测试用**本机 TCP 半关闭**制造同一
//! 分支（测试构建 + 显式 `allow_local_fixture`），断言消息里不得出现签名或 `://`。
//!
//! 全程只连 127.0.0.1，零外网、零付费。

mod common;

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use common::TestDir;
use everything_manual::assets::glb::{DownloadPolicy, ModelDownloader};

/// 复验用签名 canary（只出现在测试进程内存与本机回环请求里）。
const QA26_SIGNATURE: &str = "qa26-canary-transport-signature-7d31";

/// 极简本机“CDN”：读掉请求头/体，回 200 + 部分 body 后**半关闭**（模拟传输中断）。
/// `mode`：Truncate = 半关闭截断；BlackHole = 收下请求后不响应（模拟超时）。
struct QaHalfCloseServer {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    request_count: Arc<AtomicUsize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum QaMode {
    Truncate,
    BlackHole,
}

impl QaHalfCloseServer {
    fn start(body_len: usize, truncate_at: usize) -> Self {
        Self::start_with(QaMode::Truncate, body_len, truncate_at)
    }

    fn start_with(mode: QaMode, body_len: usize, truncate_at: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定回环端口");
        listener.set_nonblocking(true).expect("非阻塞监听");
        let port = listener.local_addr().expect("本地地址").port();
        let stop = Arc::new(AtomicBool::new(false));
        let request_count = Arc::new(AtomicUsize::new(0));
        let thread_stop = Arc::clone(&stop);
        let thread_count = Arc::clone(&request_count);
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        thread_count.fetch_add(1, Ordering::SeqCst);
                        match mode {
                            QaMode::Truncate => {
                                let _ = serve_half_close(&mut stream, body_len, truncate_at);
                            }
                            QaMode::BlackHole => {
                                // 收下连接与请求后不写任何响应字节：触发请求超时。
                                let mut scratch = [0_u8; 4096];
                                let _ = stream.read(&mut scratch);
                                std::thread::sleep(Duration::from_secs(30));
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            port,
            stop,
            thread: Some(thread),
            request_count,
        }
    }

    fn url_with_signature(&self) -> String {
        format!(
            "http://127.0.0.1:{}/qa-r26/model.glb?sign={QA26_SIGNATURE}&expires=9999999999",
            self.port
        )
    }

    fn requests(&self) -> usize {
        self.request_count.load(Ordering::SeqCst)
    }
}

impl Drop for QaHalfCloseServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve_half_close(
    stream: &mut TcpStream,
    body_len: usize,
    truncate_at: usize,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut buffer = Vec::new();
    let mut scratch = [0_u8; 4096];
    let header_end = loop {
        if let Some(position) = find_header_end(&buffer) {
            break position;
        }
        match stream.read(&mut scratch) {
            Ok(0) => return Ok(()),
            Ok(read) => buffer.extend_from_slice(&scratch[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(error) => return Err(error),
        }
    };
    // 读掉剩余 body（GET 一般没有；容错）。
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let content_length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let mut body_read = buffer.len().saturating_sub(header_end).min(content_length);
    while body_read < content_length {
        match stream.read(&mut scratch) {
            Ok(0) => break,
            Ok(read) => body_read += read,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(error) => return Err(error),
        }
    }
    // 声明完整长度（Content-Length = body_len），但只写 truncate_at 字节后断开：
    // 客户端会在读体中途收到连接关闭 → 传输错误。
    let header = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: model/gltf-binary\r\ncontent-length: {body_len}\r\nconnection: close\r\n\r\n"
    );
    stream.write_all(header.as_bytes())?;
    stream.flush()?;
    let payload = vec![0x41_u8; truncate_at];
    stream.write_all(&payload)?;
    stream.flush()?;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

fn qa26_policy() -> DownloadPolicy {
    DownloadPolicy {
        allowed_hosts: vec!["127.0.0.1".to_owned()],
        allow_local_fixture: true,
        max_bytes: 8 * 1024 * 1024,
        max_redirects: 5,
        connect_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(30),
    }
}

fn qa26_policy_fast_timeout() -> DownloadPolicy {
    DownloadPolicy {
        connect_timeout: Duration::from_secs(1),
        request_timeout: Duration::from_secs(1),
        ..qa26_policy()
    }
}

/// 断言：传输类失败消息里不得出现签名或 URL 形态字符串（判据同 BUG-008 复验）。
fn qa26_assert_message_clean(error: &everything_manual::assets::glb::DownloadError) {
    let message = error.message();
    println!("QA-R26 transport message = {message}");
    assert!(
        !message.contains(QA26_SIGNATURE),
        "阶段消息不得含签名：{message}"
    );
    assert!(
        !message.contains("://"),
        "阶段消息不得含 URL 形态字符串：{message}"
    );
}

/// 连接被拒（生产：CDN 端口不可达）的**阶段消息**必须不含签名 URL 或 `://`。
///
/// 回合 26 曾为 RED（BUG-009：`classify_transport` 拼接的 reqwest 错误带了
/// `for url (<完整签名 URL>)`）；RD 修复（R28）后 **QA 回合 27 转正为常绿回归用例**
/// （断言未改，只去掉 `#[ignore]`）。直接跑：`cargo test -p everything-manual --test qa_t20_bug008_independent`
#[tokio::test]
async fn qa26_connect_refused_message_must_not_leak_signed_url() {
    // 先占一个端口再释放：保证该端口上有极小概率被别的进程占用 → 用 listen 后关闭。
    let listener = TcpListener::bind("127.0.0.1:0").expect("占位端口");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let dir = TestDir::new("qa26-connect-refused");
    let downloader = ModelDownloader::new(dir.path(), qa26_policy_fast_timeout());
    let url = format!("http://127.0.0.1:{port}/qa-r26/model.glb?sign={QA26_SIGNATURE}");
    let error = downloader
        .download(&url)
        .await
        .expect_err("连接被拒必须失败");
    assert_eq!(error.code(), "download_transport", "{error:?}");
    qa26_assert_message_clean(&error);
}

/// 请求超时（生产：CDN 无响应）的**阶段消息**必须不含签名 URL 或 `://`。
///
/// 回合 26 曾为 RED（同 `qa26_connect_refused_*`）；RD 修复（R28）后 QA 回合 27 转正。
#[tokio::test]
async fn qa26_timeout_message_must_not_leak_signed_url() {
    let server = QaHalfCloseServer::start_with(QaMode::BlackHole, 0, 0);
    let dir = TestDir::new("qa26-timeout-message");
    let downloader = ModelDownloader::new(dir.path(), qa26_policy_fast_timeout());
    let error = downloader
        .download(&server.url_with_signature())
        .await
        .expect_err("超时必须失败");
    assert_eq!(error.code(), "download_transport", "{error:?}");
    qa26_assert_message_clean(&error);
}

/// 传输中断（生产：CDN 超时/断连）的**阶段消息**必须不含签名 URL 或 `://`。
#[tokio::test]
async fn qa26_transport_failure_message_must_not_leak_signed_url() {
    let server = QaHalfCloseServer::start(4096, 512);
    let dir = TestDir::new("qa26-transport-message");
    let downloader = ModelDownloader::new(dir.path(), qa26_policy());
    let url = server.url_with_signature();

    let error = downloader
        .download(&url)
        .await
        .expect_err("半关闭截断必须导致下载失败");
    assert_eq!(error.code(), "download_transport", "{error:?}");
    assert!(server.requests() >= 1, "fixture 必须收到请求（用例前提）");

    let message = error.message();
    println!("QA-R26 transport message = {message}");
    assert!(
        !message.contains(QA26_SIGNATURE),
        "阶段消息不得含签名：{message}"
    );
    assert!(
        !message.contains("://"),
        "阶段消息不得含 URL 形态字符串：{message}"
    );
}

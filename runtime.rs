#![allow(dead_code, unused_imports, clippy::all)]

use std::sync::Arc;
use std::sync::Mutex;

use serde::Serialize;
use serde::de::DeserializeOwned;

pub use reqwest::Method;

#[derive(Debug, Clone)]
pub enum ApiError {
  Network(String),
  Api { code: u16, message: String, body: String },
  Parse(String),
}

impl ApiError {
  fn rejected(status: reqwest::StatusCode, body: String) -> Self {
    let message = match serde_json::from_str::<serde_json::Value>(&body) {
      Ok(serde_json::Value::String(message)) => message,
      Ok(_) => status.to_string(),
      Err(_) if !body.trim().is_empty() => body.clone(),
      Err(_) => status.to_string(),
    };
    Self::Api { code: status.as_u16(), message, body }
  }
}

impl std::fmt::Display for ApiError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      ApiError::Network(m) => write!(f, "network error: {m}"),
      ApiError::Api { code, message, .. } => write!(f, "api error {code}: {message}"),
      ApiError::Parse(m) => write!(f, "parse error: {m}"),
    }
  }
}
impl std::error::Error for ApiError {}

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Clone)]
pub struct Client {
  host: String,
  http: reqwest::Client,
  headers: Arc<Mutex<reqwest::header::HeaderMap>>,
  timeout: Option<std::time::Duration>,
}

pub struct ClientBuilder {
  host: String,
  headers: reqwest::header::HeaderMap,
  timeout: Option<std::time::Duration>,
}

impl Client {
  pub fn builder(host: impl Into<String>) -> ClientBuilder {
    ClientBuilder { host: host.into().trim_end_matches('/').to_owned(), headers: reqwest::header::HeaderMap::new(), timeout: None }
  }

  pub fn set_headers(&self, headers: reqwest::header::HeaderMap) {
    *self.headers.lock().unwrap() = headers;
  }

  pub fn set_bearer(&self, token: Option<&str>) {
    let mut h = self.headers.lock().unwrap();
    match token {
      Some(t) => {
        if let Ok(v) = format!("Bearer {t}").parse() {
          h.insert(reqwest::header::AUTHORIZATION, v);
        }
      }
      None => {
        h.remove(reqwest::header::AUTHORIZATION);
      }
    }
  }

  fn default_headers(&self) -> reqwest::header::HeaderMap {
    self.headers.lock().unwrap().clone()
  }
}

impl ClientBuilder {
  pub fn header(mut self, name: &str, value: &str) -> Self {
    if let (Ok(n), Ok(v)) = (reqwest::header::HeaderName::from_bytes(name.as_bytes()), value.parse()) {
      self.headers.insert(n, v);
    }
    self
  }

  pub fn bearer(self, token: &str) -> Self {
    self.header("Authorization", &format!("Bearer {token}"))
  }

  pub fn timeout(mut self, timeout: std::time::Duration) -> Self {
    self.timeout = Some(timeout);
    self
  }

  pub fn build(self) -> Client {
    Client { host: self.host, http: reqwest::Client::new(), headers: Arc::new(Mutex::new(self.headers)), timeout: self.timeout }
  }
}

pub fn encode_path(value: &str) -> String {
  let mut out = String::with_capacity(value.len());
  for b in value.bytes() {
    match b {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
      _ => out.push_str(&format!("%{b:02X}")),
    }
  }
  out
}

impl Client {
  fn url<Q: Serialize>(&self, path: &str, query: Option<&Q>) -> ApiResult<reqwest::Url> {
    let mut s = format!("{}{}", self.host, path);
    if let Some(q) = query {
      let qs = serde_qs::to_string(q).map_err(|e| ApiError::Parse(e.to_string()))?;
      if !qs.is_empty() {
        s.push('?');
        s.push_str(&qs);
      }
    }
    reqwest::Url::parse(&s).map_err(|e| ApiError::Network(e.to_string()))
  }
}

impl Client {
  pub async fn request<Q, B, R>(&self, method: Method, path: &str, query: Option<&Q>, body: Option<&B>) -> ApiResult<R>
  where
    Q: Serialize,
    B: Serialize,
    R: DeserializeOwned,
  {
    let text = self.request_text(method, path, query, body).await?;
    serde_json::from_str(&text).map_err(|e| ApiError::Parse(e.to_string()))
  }

  pub async fn request_bytes<Q, B>(&self, method: Method, path: &str, query: Option<&Q>, body: Option<&B>) -> ApiResult<Vec<u8>>
  where
    Q: Serialize,
    B: Serialize,
  {
    let url = self.url(path, query)?;
    let mut req = self.http.request(method, url).headers(self.default_headers());
    if let Some(t) = self.timeout {
      req = req.timeout(t);
    }
    if let Some(b) = body {
      req = req.json(b);
    }
    let resp = req.send().await.map_err(|e| ApiError::Network(e.to_string()))?;
    let status = resp.status();
    let bytes = resp.bytes().await.map_err(|e| ApiError::Network(e.to_string()))?;
    if !status.is_success() {
      return Err(ApiError::rejected(status, String::from_utf8_lossy(&bytes).into_owned()));
    }
    Ok(bytes.to_vec())
  }

  pub async fn request_multipart<Q, R>(&self, method: Method, path: &str, query: Option<&Q>, form: reqwest::multipart::Form) -> ApiResult<R>
  where
    Q: Serialize,
    R: DeserializeOwned,
  {
    let url = self.url(path, query)?;
    let mut req = self.http.request(method, url).headers(self.default_headers()).multipart(form);
    if let Some(t) = self.timeout {
      req = req.timeout(t);
    }
    let resp = req.send().await.map_err(|e| ApiError::Network(e.to_string()))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| ApiError::Network(e.to_string()))?;
    if !status.is_success() {
      return Err(ApiError::rejected(status, text));
    }
    serde_json::from_str(&text).map_err(|e| ApiError::Parse(e.to_string()))
  }

  async fn request_text<Q, B>(&self, method: Method, path: &str, query: Option<&Q>, body: Option<&B>) -> ApiResult<String>
  where
    Q: Serialize,
    B: Serialize,
  {
    let url = self.url(path, query)?;
    let mut req = self.http.request(method, url).headers(self.default_headers());
    if let Some(t) = self.timeout {
      req = req.timeout(t);
    }
    if let Some(b) = body {
      req = req.json(b);
    }
    let resp = req.send().await.map_err(|e| ApiError::Network(e.to_string()))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| ApiError::Network(e.to_string()))?;
    if !status.is_success() {
      return Err(ApiError::rejected(status, text));
    }
    Ok(text)
  }
}

impl Client {
  pub async fn sse<Q, E>(&self, path: &str, query: Option<&Q>, mut on_event: impl FnMut(E)) -> ApiResult<()>
  where
    Q: Serialize,
    E: DeserializeOwned,
  {
    use futures_util::StreamExt;

    let url = self.url(path, query)?;
    let resp = self.http.get(url).headers(self.default_headers()).header(reqwest::header::ACCEPT, "text/event-stream").send().await.map_err(|e| ApiError::Network(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
      let body = resp.text().await.unwrap_or_default();
      return Err(ApiError::rejected(status, body));
    }

    let mut stream = resp.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
      let chunk = chunk.map_err(|e| ApiError::Network(e.to_string()))?;
      buffer.extend_from_slice(&chunk);
      while let Some((consumed, end)) = find_event_boundary(&buffer) {
        let event = String::from_utf8_lossy(&buffer[..end]).into_owned();
        buffer.drain(..consumed);
        if let Some(data) = event_data(&event) {
          match serde_json::from_str::<E>(&data) {
            Ok(ev) => on_event(ev),
            Err(e) => return Err(ApiError::Parse(e.to_string())),
          }
        }
      }
      if buffer.len() > MAX_SSE_EVENT_BYTES {
        return Err(ApiError::Parse(format!("sse event exceeded {MAX_SSE_EVENT_BYTES} bytes with no boundary")));
      }
    }
    Ok(())
  }
}

const MAX_SSE_EVENT_BYTES: usize = 16 * 1024 * 1024;

fn find_event_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
  [b"\r\n\r\n".as_slice(), b"\n\n".as_slice(), b"\r\r".as_slice()]
    .into_iter()
    .filter_map(|sep| find_bytes(buffer, sep).map(|at| (at + sep.len(), at)))
    .min_by_key(|(_, at)| *at)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
  haystack.windows(needle.len()).position(|window| window == needle)
}

fn event_data(event: &str) -> Option<String> {
  let mut lines = Vec::new();
  for line in event.split(['\r', '\n']).filter(|l| !l.is_empty()) {
    if line.starts_with(':') {
      continue;
    }
    if let Some(rest) = line.strip_prefix("data:") {
      lines.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
    }
  }
  if lines.is_empty() { None } else { Some(lines.join("\n")) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsStatus {
  Connecting,
  Open,
  Reconnecting,
  Stopped,
}

pub struct WsHandlers<R> {
  pub on_message: Option<Box<dyn FnMut(R) + Send>>,
  pub on_binary: Option<Box<dyn FnMut(Vec<u8>) + Send>>,
  pub on_open: Option<Box<dyn FnMut() + Send>>,
  pub on_close: Option<Box<dyn FnMut() + Send>>,
  pub on_error: Option<Box<dyn FnMut(String) + Send>>,
  pub on_status: Option<Box<dyn FnMut(WsStatus) + Send>>,
}

impl<R> Default for WsHandlers<R> {
  fn default() -> Self {
    Self { on_message: None, on_binary: None, on_open: None, on_close: None, on_error: None, on_status: None }
  }
}

enum WsFrame {
  Text(String),
  Binary(Vec<u8>),
}

const WS_SEND_CAPACITY: usize = 256;

pub struct WsConnection<S> {
  tx: tokio::sync::mpsc::Sender<WsFrame>,
  stop: Arc<std::sync::atomic::AtomicBool>,
  wake: Arc<tokio::sync::Notify>,
  _send: std::marker::PhantomData<fn(S)>,
}

impl<S: Serialize> WsConnection<S> {
  pub fn send(&self, message: &S) -> bool {
    match serde_json::to_string(message) {
      Ok(text) => self.enqueue(WsFrame::Text(text)),
      Err(_) => false,
    }
  }

  pub fn send_binary(&self, data: Vec<u8>) -> bool {
    self.enqueue(WsFrame::Binary(data))
  }

  fn enqueue(&self, frame: WsFrame) -> bool {
    if self.stop.load(std::sync::atomic::Ordering::SeqCst) {
      return false;
    }
    self.tx.try_send(frame).is_ok()
  }

  pub fn stop(&self) {
    self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
    self.wake.notify_one();
  }
}

const KEEPALIVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

impl Client {
  pub fn ws<S, R, Q>(&self, path: &str, query: Option<&Q>, mut handlers: WsHandlers<R>) -> WsConnection<S>
  where
    S: Serialize + Send + 'static,
    R: DeserializeOwned + Send + 'static,
    Q: Serialize,
  {
    use futures_util::SinkExt;
    use futures_util::StreamExt;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<WsFrame>(WS_SEND_CAPACITY);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let wake = Arc::new(tokio::sync::Notify::new());
    let headers = self.headers.clone();
    let stop_task = stop.clone();
    let wake_task = wake.clone();

    let prepared = ws_url(&self.host, path, query).map_err(|e| e.to_string()).and_then(|url| WsTls::new().map(|tls| (url, tls)));
    let (url, tls) = match prepared {
      Ok(prepared) => prepared,
      Err(e) => {
        if let Some(cb) = handlers.on_error.as_mut() {
          cb(e);
        }
        if let Some(cb) = handlers.on_status.as_mut() {
          cb(WsStatus::Stopped);
        }
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        return WsConnection { tx, stop, wake, _send: std::marker::PhantomData };
      }
    };

    tokio::spawn(async move {
      let mut failures: u32 = 0;
      let emit_status = |h: &mut WsHandlers<R>, s: WsStatus| {
        if let Some(cb) = h.on_status.as_mut() {
          cb(s);
        }
      };
      loop {
        if stop_task.load(std::sync::atomic::Ordering::SeqCst) {
          break;
        }
        emit_status(&mut handlers, if failures == 0 { WsStatus::Connecting } else { WsStatus::Reconnecting });
        let request = {
          use tokio_tungstenite::tungstenite::client::IntoClientRequest;
          match url.clone().into_client_request() {
            Ok(mut request) => {
              for (name, value) in headers.lock().unwrap().iter() {
                request.headers_mut().insert(name.clone(), value.clone());
              }
              request
            }
            Err(e) => {
              if let Some(cb) = handlers.on_error.as_mut() {
                cb(e.to_string());
              }
              break;
            }
          }
        };
        match tls.connect(request).await {
          Ok((ws_stream, _)) => {
            failures = 0;
            emit_status(&mut handlers, WsStatus::Open);
            if let Some(cb) = handlers.on_open.as_mut() {
              cb();
            }
            let (mut write, mut read) = ws_stream.split();
            let mut keepalive = tokio::time::interval(KEEPALIVE_INTERVAL);
            keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            keepalive.tick().await;

            let mut closed_by_us = false;
            {
              let writer = async {
                loop {
                  tokio::select! {
                    _ = keepalive.tick() => {
                      if write.send(tokio_tungstenite::tungstenite::Message::Ping(Vec::new().into())).await.is_err() { return false; }
                    }
                    outgoing = rx.recv() => match outgoing {
                      Some(WsFrame::Text(text)) => { if write.send(tokio_tungstenite::tungstenite::Message::Text(text.into())).await.is_err() { return false; } }
                      Some(WsFrame::Binary(data)) => { if write.send(tokio_tungstenite::tungstenite::Message::Binary(data.into())).await.is_err() { return false; } }
                      None => { let _ = write.close().await; return true; }
                    },
                    _ = wake_task.notified() => { let _ = write.close().await; return true; }
                  }
                }
              };
              let reader = async {
                while let Some(incoming) = read.next().await {
                  match incoming {
                    Ok(tokio_tungstenite::tungstenite::Message::Text(t)) => match serde_json::from_str::<R>(&t) {
                      Ok(msg) => { if let Some(cb) = handlers.on_message.as_mut() { cb(msg); } }
                      Err(e) => { if let Some(cb) = handlers.on_error.as_mut() { cb(e.to_string()); } }
                    },
                    Ok(tokio_tungstenite::tungstenite::Message::Binary(data)) => { if let Some(cb) = handlers.on_binary.as_mut() { cb(data.into()); } }
                    Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => break,
                    Ok(tokio_tungstenite::tungstenite::Message::Ping(_)) | Ok(tokio_tungstenite::tungstenite::Message::Pong(_)) | Ok(tokio_tungstenite::tungstenite::Message::Frame(_)) => {}
                    Err(e) => { if let Some(cb) = handlers.on_error.as_mut() { cb(e.to_string()); } break; }
                  }
                }
              };
              tokio::select! {
                manual = writer => closed_by_us = manual,
                _ = reader => {}
              }
            }
            if closed_by_us {
              stop_task.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            if let Some(cb) = handlers.on_close.as_mut() {
              cb();
            }
          }
          Err(e) => {
            if let Some(cb) = handlers.on_error.as_mut() {
              cb(e.to_string());
            }
          }
        }
        if stop_task.load(std::sync::atomic::Ordering::SeqCst) {
          break;
        }
        while rx.try_recv().is_ok() {}
        let backoff = std::cmp::min(15_000u64, 500u64.saturating_mul(1 << failures.min(5)));
        failures = failures.saturating_add(1);
        tokio::select! {
          _ = tokio::time::sleep(std::time::Duration::from_millis(backoff)) => {}
          _ = wake_task.notified() => break,
        }
      }
      emit_status(&mut handlers, WsStatus::Stopped);
    });

    WsConnection { tx, stop, wake, _send: std::marker::PhantomData }
  }
}

struct WsTls {
  #[cfg(feature = "rustls-tls")]
  config: Arc<rustls::ClientConfig>,
}

impl WsTls {
  #[cfg(feature = "rustls-tls")]
  fn new() -> Result<Self, String> {
    use rustls_platform_verifier::BuilderVerifierExt;
    let provider = rustls::crypto::CryptoProvider::get_default().cloned().unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()));
    let config = rustls::ClientConfig::builder_with_provider(provider)
      .with_safe_default_protocol_versions()
      .and_then(|builder| builder.with_platform_verifier())
      .map_err(|e| e.to_string())?
      .with_no_client_auth();
    Ok(Self { config: Arc::new(config) })
  }

  #[cfg(not(feature = "rustls-tls"))]
  fn new() -> Result<Self, String> {
    Ok(Self {})
  }

  async fn connect(
    &self,
    request: tokio_tungstenite::tungstenite::handshake::client::Request,
  ) -> Result<(tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, tokio_tungstenite::tungstenite::handshake::client::Response), tokio_tungstenite::tungstenite::Error> {
    #[cfg(feature = "rustls-tls")]
    return tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(tokio_tungstenite::Connector::Rustls(self.config.clone()))).await;
    #[cfg(not(feature = "rustls-tls"))]
    return tokio_tungstenite::connect_async(request).await;
  }
}

fn ws_url<Q: Serialize>(host: &str, path: &str, query: Option<&Q>) -> ApiResult<String> {
  let base = if let Some(rest) = host.strip_prefix("https://") {
    format!("wss://{rest}")
  } else if let Some(rest) = host.strip_prefix("http://") {
    format!("ws://{rest}")
  } else {
    host.to_string()
  };
  let mut url = format!("{base}{path}");
  if let Some(q) = query {
    let qs = serde_qs::to_string(q).map_err(|e| ApiError::Parse(e.to_string()))?;
    if !qs.is_empty() {
      url.push('?');
      url.push_str(&qs);
    }
  }
  Ok(url)
}

//! JetStream's documented JSON API over the official async NATS connection.
use super::{Envelope, Publisher, Receipt};
use crate::shared::{
    errors::{Failure, Kind},
    secret::SecretString,
};
use async_nats::{Client, HeaderMap, Message};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    future::Future,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
pub struct Config {
    pub url: SecretString,
    pub stream: String,
    pub consumer: String,
    pub timeout: Duration,
}
pub struct Broker {
    client: Client,
    config: Config,
    subject: String,
    closed: AtomicBool,
}
fn unavailable() -> Failure {
    Failure::new(Kind::Unavailable, "JetStream operation incomplete")
        .with_type("events.jetstream_unavailable")
}
fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid JetStream configuration")
        .with_type("events.jetstream_config")
}
fn name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn compatible(got: &Value, want: &Value) -> bool {
    want.as_object()
        .unwrap()
        .iter()
        .all(|(k, v)| got.get(k) == Some(v))
}
impl Broker {
    pub async fn open(config: Config) -> Result<Self, Failure> {
        let url = url::Url::parse(config.url.reveal()).map_err(|_| invalid())?;
        if !matches!(url.scheme(), "nats" | "tls")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.path().is_empty()
            || !name(&config.stream)
            || !name(&config.consumer)
            || config.timeout < Duration::from_millis(1)
            || config.timeout > Duration::from_secs(5)
        {
            return Err(invalid());
        }
        let client = tokio::time::timeout(
            config.timeout,
            async_nats::ConnectOptions::new()
                .connection_timeout(config.timeout)
                .request_timeout(Some(config.timeout))
                .max_reconnects(0)
                .connect(config.url.reveal()),
        )
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?;
        Ok(Self {
            client,
            subject: format!("n2f.events.{}", config.stream),
            config,
            closed: AtomicBool::new(false),
        })
    }
    pub async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let _ = tokio::time::timeout(self.config.timeout, self.client.drain()).await;
    }
    async fn bounded<T>(
        &self,
        work: impl Future<Output = Result<T, Failure>>,
    ) -> Result<T, Failure> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(unavailable());
        }
        tokio::time::timeout(self.config.timeout, work)
            .await
            .map_err(|_| unavailable())?
    }
    async fn request(
        &self,
        subject: String,
        data: Vec<u8>,
        headers: Option<HeaderMap>,
    ) -> Result<Message, Failure> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(unavailable());
        }
        // Pull deliveries carry the original event subject, not the request inbox.
        // A dedicated subscription avoids request-multiplexer subject matching.
        let mut request = async_nats::Request::new()
            .payload(data.into())
            .inbox(self.client.new_inbox());
        if let Some(h) = headers {
            request = request.headers(h);
        }
        let message = self
            .client
            .send_request(subject, request)
            .await
            .map_err(|_| unavailable())?;
        if message.payload.len() > 131072 {
            return Err(unavailable());
        }
        Ok(message)
    }
    async fn api(&self, subject: String, data: Value) -> Result<Value, Failure> {
        let message = self
            .request(
                subject,
                serde_json::to_vec(&data).map_err(|_| invalid())?,
                None,
            )
            .await?;
        let value: Value = serde_json::from_slice(&message.payload).map_err(|_| unavailable())?;
        if !value.is_object() {
            return Err(unavailable());
        }
        Ok(value)
    }
    pub async fn provision(&self) -> Result<(), Failure> {
        self.bounded(async {
  let stream=json!({"name":self.config.stream,"subjects":[self.subject],"storage":"file","num_replicas":1,"retention":"limits","discard":"new","max_bytes":67108864,"max_msg_size":131072,"duplicate_window":120000000000u64});
  let mut info=self.api(format!("$JS.API.STREAM.INFO.{}",self.config.stream),json!({})).await?;
  if info["error"]["code"]==404 {info=self.api(format!("$JS.API.STREAM.CREATE.{}",self.config.stream),stream.clone()).await?;}
  if info.get("error").is_some(){return Err(unavailable())}
  if !compatible(&info["config"],&stream){return Err(invalid())}
  let consumer=json!({"durable_name":self.config.consumer,"ack_policy":"explicit","ack_wait":1000000000,"max_deliver":5,"max_ack_pending":1,"filter_subject":self.subject,"deliver_policy":"all","replay_policy":"instant"});
  info=self.api(format!("$JS.API.CONSUMER.INFO.{}.{}",self.config.stream,self.config.consumer),json!({})).await?;
  if info["error"]["code"]==404 {info=self.api(format!("$JS.API.CONSUMER.DURABLE.CREATE.{}.{}",self.config.stream,self.config.consumer),json!({"stream_name":self.config.stream,"config":consumer})).await?;}
  if info.get("error").is_some(){return Err(unavailable())}
  if !compatible(&info["config"],&consumer){return Err(invalid())}Ok(())
 }).await
    }
    pub async fn transfer(&self, sink: &dyn Publisher) -> Result<bool, Failure> {
        self.bounded(async {
            let msg = self
                .request(
                    format!(
                        "$JS.API.CONSUMER.MSG.NEXT.{}.{}",
                        self.config.stream, self.config.consumer
                    ),
                    br#"{"batch":1,"no_wait":true}"#.to_vec(),
                    None,
                )
                .await?;
            if msg
                .status
                .is_some_and(|s| s.as_u16() == 404 || s.as_u16() == 408)
            {
                return Ok(false);
            }
            let reply = msg.reply.ok_or_else(unavailable)?;
            let event = Envelope::decode(&msg.payload)?;
            let receipt = sink.publish(&event).await?;
            if !receipt.durable || receipt.event_id != event.id() {
                return Err(unavailable());
            }
            self.request(reply.to_string(), b"+ACK".to_vec(), None)
                .await?;
            Ok(true)
        })
        .await
    }
}
impl Publisher for Broker {
    fn publish<'a>(&'a self, event: &'a Envelope) -> BoxFuture<'a, Result<Receipt, Failure>> {
        Box::pin(async move {
            self.bounded(async {
                let mut headers = HeaderMap::new();
                headers.insert("Nats-Msg-Id", event.id().to_string());
                headers.insert("Nats-Expected-Stream", self.config.stream.clone());
                let message = self
                    .request(self.subject.clone(), event.bytes().to_vec(), Some(headers))
                    .await?;
                let ack: Value =
                    serde_json::from_slice(&message.payload).map_err(|_| unavailable())?;
                if ack.get("error").is_some()
                    || ack["stream"] != self.config.stream
                    || ack["seq"].as_u64().unwrap_or(0) == 0
                {
                    return Err(unavailable());
                }
                if ack["duplicate"] == true {
                    let stored = self
                        .api(
                            format!("$JS.API.STREAM.MSG.GET.{}", self.config.stream),
                            json!({"seq":ack["seq"]}),
                        )
                        .await?;
                    let raw = STANDARD
                        .decode(stored["message"]["data"].as_str().ok_or_else(unavailable)?)
                        .map_err(|_| unavailable())?;
                    let original: Value =
                        serde_json::from_slice(&raw).map_err(|_| unavailable())?;
                    let current: Value =
                        serde_json::from_slice(event.bytes()).map_err(|_| unavailable())?;
                    if original != current {
                        return Err(Failure::new(Kind::Conflict, "event delivery conflict")
                            .with_type("events.id_reused"));
                    }
                }
                Ok(Receipt {
                    event_id: event.id(),
                    durable: true,
                })
            })
            .await
        })
    }
}

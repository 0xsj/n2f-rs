//! Axum upgrade/session ownership. JSON HTTP completion does not own upgraded sockets.
use super::{Message as Envelope, decode, encode};
use crate::shared::errors::Failure;
use ::axum::{
    extract::ws::{CloseFrame, Message, WebSocket},
    extract::{State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::future::BoxFuture;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
pub struct Session {
    pub handle:
        Arc<dyn Fn(Envelope) -> BoxFuture<'static, Result<Envelope, Failure>> + Send + Sync>,
    pub close: Box<dyn FnOnce() + Send>,
}
pub struct Server {
    origin: String,
    open: Arc<dyn Fn() -> Result<Session, Failure> + Send + Sync>,
    closed: AtomicBool,
    slots: Arc<tokio::sync::Semaphore>,
    stop: tokio::sync::watch::Sender<bool>,
}
impl Server {
    pub fn new(
        origin: String,
        open: Arc<dyn Fn() -> Result<Session, Failure> + Send + Sync>,
    ) -> Self {
        Self {
            origin,
            open,
            closed: AtomicBool::new(false),
            slots: Arc::new(tokio::sync::Semaphore::new(64)),
            stop: tokio::sync::watch::channel(false).0,
        }
    }
    pub async fn upgrade(
        State(server): State<Arc<Self>>,
        headers: HeaderMap,
        ws: WebSocketUpgrade,
    ) -> Response {
        let origins: Vec<_> = headers.get_all("origin").iter().collect();
        let protocol = headers.get_all("sec-websocket-protocol").iter().any(|v| {
            v.to_str()
                .is_ok_and(|v| v.split(',').any(|p| p.trim() == "n2f.v1"))
        });
        if origins.len() != 1
            || origins[0].to_str().ok() != Some(server.origin.as_str())
            || !protocol
        {
            return StatusCode::FORBIDDEN.into_response();
        }
        if server.closed.load(Ordering::SeqCst) {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        let Ok(permit) = server.slots.clone().try_acquire_owned() else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        ws.protocols(["n2f.v1"])
            .max_message_size(65536)
            .max_frame_size(65536)
            .on_upgrade(move |socket| async move {
                let _permit = permit;
                server.run(socket).await
            })
            .into_response()
    }
    async fn run(&self, mut socket: WebSocket) {
        let Ok(session) = (self.open)() else {
            close(&mut socket, 1011).await;
            return;
        };
        let mut stop = self.stop.subscribe();
        let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
        heartbeat.tick().await;
        let mut pong = true;
        if self.closed.load(Ordering::SeqCst) {
            close(&mut socket, 1001).await;
            (session.close)();
            return;
        }
        loop {
            tokio::select! {
                   _=stop.changed()=>{close(&mut socket,1001).await;break;},
                   _=heartbeat.tick()=>{if !pong{break;}pong=false;if !matches!(tokio::time::timeout(Duration::from_secs(1),socket.send(Message::Ping(vec![].into()))).await,Ok(Ok(()))){break;}},
                   incoming=socket.recv()=>{
                    match incoming{
                     Some(Ok(Message::Pong(_)))=>pong=true,
                     Some(Ok(Message::Ping(_)))=>{},
                     Some(Ok(Message::Text(raw)))=>{
                      let Ok(message)=decode(raw.as_bytes())else{close(&mut socket,1008).await;break;};
                      let response=tokio::select!{_=stop.changed()=>{close(&mut socket,1001).await;break;},r=tokio::time::timeout(Duration::from_secs(1),(session.handle)(message))=>r};
                      let Ok(Ok(response))=response else{close(&mut socket,1011).await;break;};let Ok(body)=encode(response)else{close(&mut socket,1011).await;break;};
                      if !matches!(tokio::time::timeout(Duration::from_secs(1),socket.send(Message::Text(body.into()))).await,Ok(Ok(()))){break;}
                     },
                     Some(Ok(Message::Binary(_)))=>{close(&mut socket,1003).await;break;},
            Some(Err(error))=>{let error=error.into_inner();let code=match error.downcast_ref::<tungstenite::Error>(){Some(tungstenite::Error::Capacity(_))=>1009,Some(tungstenite::Error::Utf8(_))=>1007,_=>1002};close(&mut socket,code).await;break;},
            _=>break,
                    }
                   }
                  }
        }
        (session.close)();
    }
    pub async fn close(&self, budget: Duration) -> bool {
        self.closed.store(true, Ordering::SeqCst);
        self.stop.send_replace(true);
        tokio::time::timeout(budget, async {
            let _all = self.slots.acquire_many(64).await;
        })
        .await
        .is_ok()
    }
}
async fn close(socket: &mut WebSocket, code: u16) {
    let _ = tokio::time::timeout(
        Duration::from_millis(100),
        socket.send(Message::Close(Some(CloseFrame {
            code,
            reason: "connection closed".into(),
        }))),
    )
    .await;
}

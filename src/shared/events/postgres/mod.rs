//! Postgres outbox and durable local mailbox; SQL handles stay in infrastructure.
use super::{Envelope, Publisher, Receipt};
use crate::shared::{
    errors::{Failure, Kind},
    id::Id,
    postgres::{Database, Migration, map},
};
use futures_util::future::BoxFuture;
use sqlx::{Connection, PgConnection, Row};
use std::sync::Arc;
pub fn migration(version: i64) -> Migration {
    Migration {
        version,
        sql: include_str!("migrations/0001_events.sql").into(),
    }
}
pub struct Store {
    pub database: Arc<Database>,
}
pub struct Mailbox {
    pub database: Arc<Database>,
}
fn conflict(code: &str) -> Failure {
    Failure::new(Kind::Conflict, "event delivery conflict").with_type(format!("events.{code}"))
}
pub async fn enqueue(tx: &mut PgConnection, event: &Envelope) -> Result<(), Failure> {
    insert(tx, "outbox", event).await
}
async fn insert(tx: &mut PgConnection, table: &str, event: &Envelope) -> Result<(), Failure> {
    let query = format!(
        "INSERT INTO public.n2f_{table} AS t(event_id,envelope) VALUES($1::uuid,$2) ON CONFLICT(event_id) DO UPDATE SET envelope=t.envelope WHERE t.envelope::jsonb=excluded.envelope::jsonb"
    );
    let affected = sqlx::query(&query)
        .bind(event.id().to_string())
        .bind(std::str::from_utf8(event.bytes()).map_err(|_| conflict("invalid"))?)
        .execute(tx)
        .await
        .map_err(map)?
        .rows_affected();
    if affected != 1 {
        return Err(conflict("id_reused"));
    }
    Ok(())
}
impl Publisher for Mailbox {
    fn publish<'a>(&'a self, event: &'a Envelope) -> BoxFuture<'a, Result<Receipt, Failure>> {
        Box::pin(async move {
            let owned = event.clone();
            self.database
                .transaction(move |tx| Box::pin(async move { insert(tx, "mailbox", &owned).await }))
                .await?;
            Ok(Receipt {
                event_id: event.id(),
                durable: true,
            })
        })
    }
}
pub struct Lease {
    pub event: Envelope,
    pub token: Id,
    pub attempt: i32,
}
impl Store {
    pub async fn claim(&self, token: Id) -> Result<Option<Lease>, Failure> {
        self.database.transaction(move|tx|Box::pin(async move{
  sqlx::query("UPDATE public.n2f_outbox SET state='dead',lease=NULL,lease_until=NULL WHERE state='pending' AND attempts=5 AND (lease_until IS NULL OR lease_until<=clock_timestamp())").execute(&mut *tx).await.map_err(map)?;
  let row=sqlx::query("WITH candidate AS (SELECT event_id FROM public.n2f_outbox WHERE state='pending' AND attempts<5 AND available_at<=clock_timestamp() AND (lease_until IS NULL OR lease_until<=clock_timestamp()) ORDER BY available_at,event_id FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE public.n2f_outbox o SET lease=$1::uuid,lease_until=clock_timestamp()+interval '30 seconds',attempts=o.attempts+1 FROM candidate c WHERE o.event_id=c.event_id RETURNING o.envelope::text AS envelope,o.attempts").bind(token.to_string()).fetch_optional(tx).await.map_err(map)?;
  row.map(|row|Ok(Lease{event:Envelope::decode(row.get::<String,_>("envelope").as_bytes())?,token,attempt:row.get("attempts")})).transpose()
 })).await
    }
    pub async fn ack(&self, lease: &Lease) -> Result<(), Failure> {
        self.finish(lease, true).await
    }
    pub async fn release(&self, lease: &Lease) -> Result<(), Failure> {
        self.finish(lease, false).await
    }
    async fn finish(&self, lease: &Lease, success: bool) -> Result<(), Failure> {
        let id = lease.event.id().to_string();
        let token = lease.token.to_string();
        self.database.transaction(move|tx|Box::pin(async move{let state=if success{"'sent'"}else{"CASE WHEN attempts>=5 THEN 'dead' ELSE 'pending' END"};let query=format!("UPDATE public.n2f_outbox SET state={state},lease=NULL,lease_until=NULL,available_at=clock_timestamp()+interval '100 milliseconds' WHERE event_id=$1::uuid AND lease=$2::uuid AND state='pending' AND lease_until>clock_timestamp()");if sqlx::query(&query).bind(id).bind(token).execute(tx).await.map_err(map)?.rows_affected()!=1{return Err(conflict("lease_lost"));}Ok(())})).await
    }
    pub async fn dispatch(&self, publisher: &dyn Publisher, token: Id) -> Result<bool, Failure> {
        let Some(lease) = self.claim(token).await? else {
            return Ok(false);
        };
        let receipt = publisher.publish(&lease.event).await;
        match receipt {
            Ok(r) if r.durable && r.event_id == lease.event.id() => {
                self.ack(&lease).await?;
                Ok(true)
            }
            other => {
                self.release(&lease).await?;
                Err(match other {
                    Err(e) => e,
                    _ => Failure::new(Kind::Unavailable, "publisher receipt invalid")
                        .with_type("events.invalid_receipt"),
                })
            }
        }
    }
}
impl Mailbox {
    pub async fn consume(
        &self,
        f: impl for<'c> FnOnce(&'c mut PgConnection, Envelope) -> BoxFuture<'c, Result<(), Failure>>
        + Send
        + 'static,
    ) -> Result<bool, Failure> {
        let (found,rejected)=self.database.transaction(move|tx|Box::pin(async move{
   let row=sqlx::query("SELECT envelope::text AS envelope,attempts FROM public.n2f_mailbox WHERE state='pending' AND attempts<5 AND available_at<=clock_timestamp() ORDER BY available_at,event_id FOR UPDATE SKIP LOCKED LIMIT 1").fetch_optional(&mut *tx).await.map_err(map)?;let Some(row)=row else{return Ok((false,None));};let event=Envelope::decode(row.get::<String,_>("envelope").as_bytes())?;let id=event.id().to_string();let attempt:i32=row.get("attempts");
   let mut sub=tx.begin().await.map_err(map)?;let rejected=f(&mut sub,event).await.err();if rejected.is_some(){sub.rollback().await.map_err(map)?;}else{sub.commit().await.map_err(map)?;}
   let state=if rejected.is_none(){"processed"}else if attempt+1>=5{"dead"}else{"pending"};sqlx::query("UPDATE public.n2f_mailbox SET state=$2,attempts=attempts+1,available_at=clock_timestamp()+interval '100 milliseconds' WHERE event_id=$1::uuid").bind(id).bind(state).execute(tx).await.map_err(map)?;Ok((true,rejected))
  })).await?;
        match rejected {
            Some(e) => Err(e),
            None => Ok(found),
        }
    }
}

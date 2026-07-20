//! Supabase Realtime (Phoenix channels) client for the live Challenge lobby.
//!
//! One WebSocket per lobby (topic `realtime:<code>`). Presence gives the live
//! player list; broadcast carries game events. Runs as a tokio task and talks to
//! the app over mpsc channels. Protocol verified against the live project — see
//! the join/heartbeat/presence/broadcast shapes below.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio_tungstenite::tungstenite::Message;

use crate::supa::{SUPABASE_KEY, SUPABASE_URL};

const HEARTBEAT: Duration = Duration::from_secs(25);

/// Commands the app sends to the transport.
pub enum RtCommand {
    Broadcast { event: String, payload: Value },
    UpdatePresence(Value),
    Close,
}

/// One tracked member of the lobby (from Realtime presence).
#[derive(Clone, Debug)]
pub struct PresenceEntry {
    pub name: String,
    /// True while this member is waiting out a running game (a late joiner).
    pub spectating: bool,
}

/// Events the transport delivers to the app.
pub enum RtEvent {
    /// The current lobby roster (from presence).
    Presence(Vec<PresenceEntry>),
    /// A broadcast game event: `(event, payload)`.
    Broadcast {
        event: String,
        payload: Value,
    },
    Disconnected(String),
}

/// Cheap-to-clone handle for driving a joined channel.
#[derive(Clone)]
pub struct RtHandle {
    tx: Sender<RtCommand>,
}

impl RtHandle {
    pub fn broadcast(&self, event: &str, payload: Value) {
        let _ = self.tx.try_send(RtCommand::Broadcast {
            event: event.to_string(),
            payload,
        });
    }

    pub fn update_presence(&self, state: Value) {
        let _ = self.tx.try_send(RtCommand::UpdatePresence(state));
    }

    pub fn close(&self) {
        let _ = self.tx.try_send(RtCommand::Close);
    }
}

/// Connect to `realtime:<topic>`, track our presence, and start the loop.
pub async fn join(topic: &str, presence: Value) -> Result<(RtHandle, Receiver<RtEvent>)> {
    let host = SUPABASE_URL.trim_start_matches("https://");
    let url = format!("wss://{host}/realtime/v1/websocket?apikey={SUPABASE_KEY}&vsn=1.0.0");
    let (ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .context("connecting to Realtime")?;

    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let (evt_tx, evt_rx) = mpsc::channel(256);
    tokio::spawn(run(
        ws,
        format!("realtime:{topic}"),
        presence,
        cmd_rx,
        evt_tx,
    ));
    Ok((RtHandle { tx: cmd_tx }, evt_rx))
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn run(
    ws: Ws,
    topic: String,
    presence: Value,
    mut cmd_rx: Receiver<RtCommand>,
    evt_tx: Sender<RtEvent>,
) {
    let (mut sink, mut stream) = ws.split();
    let mut refs = 0u64;
    let mut next_ref = || {
        refs += 1;
        refs.to_string()
    };

    // Join the channel with presence + broadcast enabled.
    let join = json!({
        "topic": topic,
        "event": "phx_join",
        "payload": {"config": {
            "broadcast": {"self": true, "ack": false},
            "presence": {"key": ""},
            "postgres_changes": []
        }, "access_token": SUPABASE_KEY},
        "ref": next_ref(),
        "join_ref": "1",
    });
    let _ = sink.send(Message::Text(join.to_string().into())).await;

    // Announce ourselves.
    let track = json!({
        "topic": topic,
        "event": "presence",
        "payload": {"type": "presence", "event": "track", "payload": presence},
        "ref": next_ref(),
    });
    let _ = sink.send(Message::Text(track.to_string().into())).await;

    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut players: BTreeMap<String, PresenceEntry> = BTreeMap::new(); // presence key -> member

    loop {
        tokio::select! {
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(t))) => {
                    if let Ok(v) = serde_json::from_str::<Value>(&t) {
                        handle_message(&v, &mut players, &evt_tx).await;
                    }
                }
                Some(Ok(Message::Ping(p))) => {
                    let _ = sink.send(Message::Pong(p)).await;
                }
                Some(Ok(Message::Close(_))) | None => {
                    let _ = evt_tx.send(RtEvent::Disconnected("connection closed".into())).await;
                    break;
                }
                Some(Err(e)) => {
                    let _ = evt_tx.send(RtEvent::Disconnected(e.to_string())).await;
                    break;
                }
                _ => {}
            },
            cmd = cmd_rx.recv() => match cmd {
                Some(RtCommand::Broadcast { event, payload }) => {
                    let m = json!({
                        "topic": topic, "event": "broadcast",
                        "payload": {"type": "broadcast", "event": event, "payload": payload},
                        "ref": next_ref(),
                    });
                    let _ = sink.send(Message::Text(m.to_string().into())).await;
                }
                Some(RtCommand::UpdatePresence(state)) => {
                    let m = json!({
                        "topic": topic, "event": "presence",
                        "payload": {"type": "presence", "event": "track", "payload": state},
                        "ref": next_ref(),
                    });
                    let _ = sink.send(Message::Text(m.to_string().into())).await;
                }
                Some(RtCommand::Close) | None => {
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            },
            _ = heartbeat.tick() => {
                let m = json!({"topic": "phoenix", "event": "heartbeat", "payload": {}, "ref": next_ref()});
                let _ = sink.send(Message::Text(m.to_string().into())).await;
            }
        }
    }
}

/// Extract the tracked member from a presence entry. Re-tracking (a presence
/// update) appends a meta, so the **last** one is the member's current state.
fn meta_entry(entry: &Value) -> Option<PresenceEntry> {
    let m = entry.get("metas").and_then(Value::as_array)?.last()?;
    let name = m.get("name").and_then(|n| n.as_str())?.to_string();
    let spectating = m
        .get("spectating")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(PresenceEntry { name, spectating })
}

async fn handle_message(
    v: &Value,
    players: &mut BTreeMap<String, PresenceEntry>,
    evt_tx: &Sender<RtEvent>,
) {
    match v.get("event").and_then(Value::as_str).unwrap_or("") {
        "presence_state" => {
            players.clear();
            if let Some(map) = v.get("payload").and_then(Value::as_object) {
                for (key, entry) in map {
                    if let Some(member) = meta_entry(entry) {
                        players.insert(key.clone(), member);
                    }
                }
            }
            emit_players(players, evt_tx).await;
        }
        "presence_diff" => {
            // Apply leaves first, then joins: a presence *update* arrives as a
            // leave of the old ref + a join of the new, and the new state must win.
            if let Some(leaves) = v
                .get("payload")
                .and_then(|p| p.get("leaves"))
                .and_then(Value::as_object)
            {
                for key in leaves.keys() {
                    players.remove(key);
                }
            }
            if let Some(joins) = v
                .get("payload")
                .and_then(|p| p.get("joins"))
                .and_then(Value::as_object)
            {
                for (key, entry) in joins {
                    if let Some(member) = meta_entry(entry) {
                        players.insert(key.clone(), member);
                    }
                }
            }
            emit_players(players, evt_tx).await;
        }
        "broadcast" => {
            let payload = v.get("payload");
            let event = payload
                .and_then(|p| p.get("event"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let inner = payload
                .and_then(|p| p.get("payload"))
                .cloned()
                .unwrap_or(Value::Null);
            let _ = evt_tx
                .send(RtEvent::Broadcast {
                    event,
                    payload: inner,
                })
                .await;
        }
        _ => {}
    }
}

async fn emit_players(players: &BTreeMap<String, PresenceEntry>, evt_tx: &Sender<RtEvent>) {
    let roster: Vec<PresenceEntry> = players.values().cloned().collect();
    let _ = evt_tx.send(RtEvent::Presence(roster)).await;
}

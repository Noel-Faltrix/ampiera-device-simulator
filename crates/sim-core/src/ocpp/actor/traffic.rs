//! Frame traffic of the actor: reading and answering frames, the outgoing call queue, answers to our calls.

use chrono::Utc;
use futures_util::SinkExt;
use serde_json::{json, Value};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use uuid::Uuid;

use super::{Actor, CallKind, OutCall, Pending, FALLBACK_HEARTBEAT_S, MAX_QUEUED_CALLS};
use crate::charge_point::handlers::{self, Effect, Reply, TriggerKind};
use crate::charge_point::state::{EndedTransaction, StopReason, SIMULATOR_ID_TAG};
use crate::charge_point::status::Phase;
use crate::handle::{BoxEvent, CallOutcome, FRAME_LOG_CAPACITY};
use crate::model::{FrameDirection, FrameLogEntry};
use crate::ocpp::frames::{self, ErrorCode, Frame};
use crate::ocpp::messages::{self, BootResponse, MeterSample, StartTransactionResponse};

/// Longest frame text kept in the log and sent to the UI.
pub const MAX_LOGGED_FRAME_BYTES: usize = 16 * 1024;

/// Longest piece of central-system text (error code or description) kept in `last_error`.
pub const MAX_REMOTE_TEXT_CHARS: usize = 200;

/// At most `max` characters of `text`, with an ellipsis when something was cut.
pub fn cut_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}…")
}

/// Cuts a frame text for the log: an oversized frame must not fill memory or the event channel.
pub fn truncate_for_log(raw: &str) -> String {
    if raw.len() <= MAX_LOGGED_FRAME_BYTES {
        return raw.to_string();
    }
    let mut end = MAX_LOGGED_FRAME_BYTES;
    while !raw.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…[gekürzt, {} Bytes]", &raw[..end], raw.len())
}

impl Actor {
    // ----- websocket traffic -----

    pub(super) async fn handle_ws_message(&mut self, message: Option<Result<Message, WsError>>) {
        match message {
            None => self.connection_dropped("Die Zentrale hat die Verbindung beendet.".to_string()),
            Some(Err(error)) => {
                self.connection_dropped(format!("Die Verbindung ist abgebrochen: {error}"));
            }
            Some(Ok(Message::Text(text))) => self.handle_text(text.as_str()).await,
            Some(Ok(Message::Ping(_))) => {
                // The pong is queued by tungstenite; flushing sends it immediately instead of with the next write.
                if let Some(ws) = self.ws.as_mut() {
                    if let Err(error) = ws.flush().await {
                        self.connection_dropped(format!("Die Verbindung ist abgebrochen: {error}"));
                    }
                }
            }
            Some(Ok(Message::Close(frame))) => self.on_close_frame(frame),
            Some(Ok(Message::Binary(_))) => {
                self.state.last_error = Some(
                    "Die Zentrale hat einen Binär-Frame gesendet; OCPP-J nutzt nur Text.".into(),
                );
            }
            Some(Ok(Message::Pong(_) | Message::Frame(_))) => {}
        }
    }

    pub(super) fn on_close_frame(
        &mut self,
        frame: Option<tokio_tungstenite::tungstenite::protocol::CloseFrame>,
    ) {
        let (code, reason) = match frame {
            Some(f) => (Some(u16::from(f.code)), f.reason.to_string()),
            None => (None, String::new()),
        };
        let _ = self.events.send(BoxEvent::Closed {
            code,
            reason: reason.clone(),
        });
        let code_text = code.map_or_else(|| "ohne Code".to_string(), |c| format!("Code {c}"));
        let normal = code == Some(u16::from(CloseCode::Normal));
        let detail = if reason.is_empty() {
            String::new()
        } else {
            format!(", Grund „{reason}“")
        };
        let text = format!(
            "Die Zentrale hat die Verbindung geschlossen ({code_text}{detail}){}.",
            if normal { "" } else { " – unerwartet" }
        );
        self.on_connection_lost(Some(text));
    }

    pub(super) async fn handle_text(&mut self, text: &str) {
        self.log_frame(FrameDirection::In, text);
        match frames::parse_frame(text) {
            Ok(Frame::Call {
                id,
                action,
                payload,
            }) => self.answer_call(&id, &action, &payload).await,
            Ok(Frame::CallResult { id, payload }) => self.on_call_result(&id, payload),
            Ok(Frame::CallError {
                id,
                code,
                description,
                ..
            }) => self.on_call_error(&id, &code, &description),
            Err(error) => {
                self.state.last_error = Some(error.message.clone());
                // Only a broken CALL is answered; answering a broken result or error could start an endless
                // exchange of error frames.
                if let (true, Some(id)) = (error.is_call, error.id) {
                    let reply =
                        frames::build_call_error(&id, ErrorCode::ProtocolError, &error.message);
                    self.send_text(reply).await;
                }
            }
        }
    }

    pub(super) async fn answer_call(&mut self, id: &str, action: &str, payload: &Value) {
        let handled = handlers::handle_call(&mut self.state, action, payload, Utc::now());
        let text = match &handled.reply {
            Reply::Result(result) => frames::build_call_result(id, result),
            Reply::Error { code, description } => frames::build_call_error(id, *code, description),
        };
        if !self.send_text(text).await {
            return;
        }
        for effect in handled.effects {
            self.apply_effect(effect).await;
        }
    }

    pub(super) async fn apply_effect(&mut self, effect: Effect) {
        match effect {
            Effect::Trigger(kind) => self.queue_triggered(kind),
            Effect::Restart(reason) => self.begin_restart(reason),
            Effect::StopTransaction { ended, reason } => self.queue_stop(&ended, reason),
            Effect::StartTransaction { id_tag } => self.state.next_id_tag = id_tag,
        }
    }

    /// Writes one text frame; on failure the connection is treated as lost.
    pub(super) async fn send_text(&mut self, text: String) -> bool {
        if self.ws.is_none() {
            return false;
        }
        self.log_frame(FrameDirection::Out, &text);
        let Some(ws) = self.ws.as_mut() else {
            return false;
        };
        match ws.send(Message::text(text)).await {
            Ok(()) => true,
            Err(error) => {
                self.connection_dropped(format!("Senden fehlgeschlagen: {error}"));
                false
            }
        }
    }

    pub(super) fn log_frame(&self, direction: FrameDirection, raw: &str) {
        let entry = FrameLogEntry {
            charge_point_id: self.state.id.clone(),
            at: Utc::now(),
            direction,
            raw: truncate_for_log(raw),
        };
        if let Ok(mut log) = self.log.lock() {
            if log.len() >= FRAME_LOG_CAPACITY {
                log.pop_front();
            }
            log.push_back(entry.clone());
        }
        self.sink.frame_logged(&entry);
        // No subscriber is not an error: scenarios only listen while they run.
        let _ = self.events.send(BoxEvent::Frame(entry));
    }

    // ----- outgoing calls -----

    pub(super) fn queue_boot(&mut self) {
        let call = OutCall::new(
            CallKind::Boot,
            "BootNotification",
            messages::boot_notification(&self.state.config),
        );
        self.queue.push_front(call);
    }

    pub(super) fn status_call(&mut self) -> OutCall {
        let status = self.state.status();
        self.reported_status = Some(status);
        OutCall::new(
            CallKind::Status,
            "StatusNotification",
            messages::status_notification(status, self.box_time()),
        )
    }

    pub(super) fn meter_call(&self) -> OutCall {
        let sample = MeterSample {
            power_w: self.state.power_w(),
            energy_wh: self.state.meter_register_wh(),
            soc_pct: if self.state.config.supports_soc {
                self.state.soc_pct()
            } else {
                None
            },
        };
        let transaction_id = self.state.transaction().map(|t| t.id);
        let payload = messages::meter_values(
            transaction_id,
            self.box_time(),
            &sample,
            &self.state.sampled_data,
        );
        OutCall::new(CallKind::Meter, "MeterValues", payload)
    }

    pub(super) fn queue_triggered(&mut self, kind: TriggerKind) {
        let call = match kind {
            TriggerKind::BootNotification => OutCall::new(
                CallKind::Boot,
                "BootNotification",
                messages::boot_notification(&self.state.config),
            ),
            TriggerKind::Heartbeat => OutCall::new(CallKind::Heartbeat, "Heartbeat", json!({})),
            TriggerKind::StatusNotification => self.status_call(),
            TriggerKind::MeterValues => self.meter_call(),
        };
        self.enqueue(call);
    }

    pub(super) fn queue_stop(&mut self, ended: &EndedTransaction, reason: StopReason) {
        let payload = messages::stop_transaction(
            ended.id,
            ended.meter_stop_wh,
            &ended.id_tag,
            reason.as_str(),
            self.box_time(),
        );
        let mut call = OutCall::new(CallKind::StopTransaction, "StopTransaction", payload);
        call.keep_offline = true;
        self.enqueue(call);
    }

    /// Takes the next call that may be sent now: before the BootNotification is accepted only that one.
    pub(super) fn take_next_call(&mut self) -> Option<OutCall> {
        let index = if self.boot_accepted {
            0
        } else {
            self.queue.iter().position(|c| c.kind == CallKind::Boot)?
        };
        self.queue.remove(index)
    }

    pub(super) async fn pump(&mut self) {
        if self.ws.is_none() || self.pending.is_some() {
            return;
        }
        let Some(call) = self.take_next_call() else {
            return;
        };
        let id = Uuid::new_v4().to_string();
        let text = frames::build_call(&id, &call.action, &call.payload);
        let deadline = Instant::now() + self.timings.call_timeout;
        // The pending entry is set first so a send failure hands the call back through the lost-connection path.
        self.pending = Some(Pending { id, call, deadline });
        self.send_text(text).await;
    }

    // ----- answers to our calls -----

    pub(super) fn take_pending(&mut self, id: &str) -> Option<Pending> {
        if self.pending.as_ref().is_some_and(|p| p.id == id) {
            self.pending.take()
        } else {
            None
        }
    }

    pub(super) fn on_call_result(&mut self, id: &str, payload: Value) {
        let Some(pending) = self.take_pending(id) else {
            // Late answer to a call that already timed out; nothing waits for it any more.
            return;
        };
        let now = Instant::now();
        match pending.call.kind {
            CallKind::Boot => self.on_boot_response(&payload, now),
            CallKind::StartTransaction => self.on_start_response(&payload),
            CallKind::StopTransaction => self.state.finish_if_unplugged(),
            CallKind::Heartbeat | CallKind::Status | CallKind::Meter | CallKind::Custom => {}
        }
        if let Some(reply) = pending.call.reply {
            let _ = reply.send(CallOutcome::Result(payload));
        }
    }

    pub(super) fn on_call_error(&mut self, id: &str, code: &str, description: &str) {
        let Some(pending) = self.take_pending(id) else {
            return;
        };
        // The text comes from the central system: bounded before it reaches the UI.
        self.state.last_error = Some(format!(
            "Die Zentrale hat {} mit {} abgelehnt: {}",
            pending.call.action,
            cut_chars(code, MAX_REMOTE_TEXT_CHARS),
            cut_chars(description, MAX_REMOTE_TEXT_CHARS)
        ));
        if pending.call.kind == CallKind::Boot {
            self.schedule_boot_retry();
        }
        if pending.call.kind == CallKind::StartTransaction {
            self.on_start_failed();
        }
        if let Some(reply) = pending.call.reply {
            let _ = reply.send(CallOutcome::Error {
                code: code.to_string(),
                description: description.to_string(),
            });
        }
    }

    pub(super) async fn on_call_timeout(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let action = pending.call.action.clone();
        self.state.last_error = Some(format!(
            "Zeitlimit überschritten: auf {action} kam innerhalb von {} s keine Antwort.",
            self.timings.call_timeout.as_secs()
        ));
        let is_boot = pending.call.kind == CallKind::Boot;
        if pending.call.kind == CallKind::StartTransaction {
            self.on_start_failed();
        }
        if pending.call.keep_offline {
            self.queue.push_front(pending.call);
        } else if let Some(reply) = pending.call.reply {
            let _ = reply.send(CallOutcome::Timeout);
        }
        if is_boot {
            // Without a boot answer the session is useless; start over with a fresh connection.
            self.reconnect_now().await;
        }
    }

    pub(super) fn schedule_boot_retry(&mut self) {
        let seconds = self.state.heartbeat_interval_s.map_or(60, u64::from);
        self.boot_retry_at = Some(Instant::now() + std::time::Duration::from_secs(seconds));
    }

    pub(super) fn on_boot_response(&mut self, payload: &Value, now: Instant) {
        let response: BootResponse = match messages::read_response("BootNotification", payload) {
            Ok(response) => response,
            Err(text) => {
                self.state.last_error = Some(text);
                self.schedule_boot_retry();
                return;
            }
        };
        if response.status != "Accepted" {
            self.state.last_error = Some(format!(
                "Die Zentrale hat die Anmeldung nicht angenommen (Status {}); die Wallbox versucht es später erneut.",
                response.status
            ));
            self.schedule_boot_retry();
            return;
        }
        let interval = response
            .interval
            .filter(|i| *i > 0)
            .map(|i| i.max(self.state.min_interval_s));
        self.state.heartbeat_interval_s = interval;
        let seconds = interval.map_or(FALLBACK_HEARTBEAT_S, u64::from);
        self.next_heartbeat = Some(now + std::time::Duration::from_secs(seconds));
        self.boot_accepted = true;
        self.accepted_at = Some(now);
        self.reported_status = None;
    }

    /// A failed StartTransaction must not leave the box stuck in `Preparing`: allow a new attempt later.
    pub(super) fn on_start_failed(&mut self) {
        self.tx_start_requested = false;
        self.tx_retry_at = Some(Instant::now() + self.timings.start_retry);
    }

    pub(super) fn on_start_response(&mut self, payload: &Value) {
        let response: StartTransactionResponse =
            match messages::read_response("StartTransaction", payload) {
                Ok(response) => response,
                Err(text) => {
                    self.state.last_error = Some(text);
                    self.on_start_failed();
                    return;
                }
            };
        if response.id_tag_info.status != "Accepted" {
            self.state.last_error = Some(format!(
                "Die Zentrale hat die Transaktion nicht erlaubt (Status {}).",
                response.id_tag_info.status
            ));
            self.on_start_failed();
            return;
        }
        let tag = self.state.next_id_tag.clone();
        self.state.next_id_tag = SIMULATOR_ID_TAG.to_string();
        if self.state.phase() == Phase::Preparing && self.state.is_plugged() {
            self.state
                .begin_transaction(response.transaction_id, &tag, Utc::now());
        } else {
            // The vehicle left while the central system was still answering; close the transaction it just opened.
            let ended = EndedTransaction {
                id: response.transaction_id,
                meter_stop_wh: self.state.meter_register_wh(),
                id_tag: tag,
            };
            self.queue_stop(&ended, StopReason::EVDisconnected);
        }
    }

    // ----- after every event -----

    pub(super) async fn after_event(&mut self) {
        if self.overflowed {
            self.overflowed = false;
            self.close_socket().await;
            self.lose_connection(
                Some(format!(
                    "Es stauten sich mehr als {MAX_QUEUED_CALLS} Nachrichten; die Wallbox startet die Verbindung neu."
                )),
                true,
            );
        }
        if self.boot_accepted && self.restart.is_none() {
            self.sync_status();
            self.ensure_transaction();
        }
        self.finish_restart_if_ready().await;
        self.pump().await;
        self.publish();
    }

    pub(super) fn sync_status(&mut self) {
        if self.reported_status != Some(self.state.status()) {
            let call = self.status_call();
            self.enqueue(call);
        }
    }

    pub(super) fn ensure_transaction(&mut self) {
        if self.tx_start_requested || self.tx_retry_at.is_some() || !self.state.wants_transaction()
        {
            return;
        }
        self.tx_start_requested = true;
        let payload = messages::start_transaction(
            self.state.meter_register_wh(),
            &self.state.next_id_tag,
            self.box_time(),
        );
        self.enqueue(OutCall::new(
            CallKind::StartTransaction,
            "StartTransaction",
            payload,
        ));
    }

    pub(super) fn publish(&mut self) {
        let mut snapshot = self.state.snapshot();
        // `updated_at` marks the last change, so it must not take part in the comparison itself.
        if let Some(last) = &self.last_published {
            snapshot.updated_at = last.updated_at;
        }
        if self.last_published.as_ref() != Some(&snapshot) {
            snapshot.updated_at = Utc::now();
            self.state.updated_at = snapshot.updated_at;
            self.snapshot_tx.send_replace(snapshot.clone());
            self.last_published = Some(snapshot);
            self.throttle.mark_changed();
        }
        let now = std::time::Instant::now();
        if self.throttle.is_due(now) {
            if let Some(snapshot) = &self.last_published {
                self.sink.charge_point_updated(snapshot);
            }
            self.throttle.mark_emitted(now);
        }
    }

    pub(super) fn fail_open_calls(&mut self) {
        self.reset_connection_state();
        for call in self.queue.drain(..) {
            if let Some(reply) = call.reply {
                let _ = reply.send(CallOutcome::NotConnected);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn central_system_text_is_cut_to_the_limit() {
        let long = "ä".repeat(500);
        let cut = cut_chars(&long, MAX_REMOTE_TEXT_CHARS);
        assert_eq!(cut.chars().count(), MAX_REMOTE_TEXT_CHARS + 1);
        assert!(cut.ends_with('…'));
        assert_eq!(
            cut_chars("kurz", 200),
            "kurz",
            "counter-check: short text stays"
        );
    }

    #[test]
    fn oversized_frames_are_cut_at_a_char_boundary() {
        let raw = "ö".repeat(MAX_LOGGED_FRAME_BYTES);
        let cut = truncate_for_log(&raw);
        assert!(cut.contains("gekürzt"));
        assert!(cut.len() < raw.len());
        assert_eq!(truncate_for_log("klein"), "klein");
    }
}

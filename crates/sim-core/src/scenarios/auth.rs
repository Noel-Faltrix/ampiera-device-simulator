//! S10: wrong password. Must never run against live: the backend counts failures per IP address.

use std::time::Duration;

use tokio::sync::broadcast;
use uuid::Uuid;

use super::ctx::{ScenarioCtx, Wait};
use super::rules::RESTORE_CHECK_NAME;
use crate::handle::BoxEvent;
use crate::model::ConnectionState;
use crate::ocpp::client::{self, ConnectFailure, Secret};

/// Number of attempts with the wrong password (SCENARIOS S10).
const ATTEMPTS: usize = 3;

/// Words in a 401 response body that would tell an attacker which half of the credentials was wrong.
const HINT_WORDS: [&str; 5] = ["password", "passwort", "identity", "kennung", "unknown"];

/// Judges the 401 answers: every attempt must be 401, and the bodies must be identical and free of hints.
pub fn judge_failures(
    failures: &[ConnectFailure],
) -> (Result<String, String>, Result<String, String>) {
    let statuses: Vec<Option<u16>> = failures.iter().map(|f| f.http_status).collect();
    let all_401 = !failures.is_empty() && statuses.iter().all(|s| *s == Some(401));
    let status_result = if all_401 {
        Ok(format!(
            "{} von {} Versuchen mit HTTP 401 abgelehnt.",
            failures.len(),
            failures.len()
        ))
    } else {
        Err(format!(
            "Statuscodes der Versuche: {statuses:?}; erwartet jedes Mal 401."
        ))
    };
    // Attempts without a recorded body (none was sent) do not take part in the comparison.
    let bodies: Vec<String> = failures.iter().filter_map(|f| f.body.clone()).collect();
    let identical = bodies.windows(2).all(|w| w[0] == w[1]);
    let hint = bodies
        .iter()
        .flat_map(|b| {
            HINT_WORDS
                .iter()
                .filter(move |w| b.to_lowercase().contains(**w))
        })
        .next();
    let hint_result = match (identical, hint) {
        (true, None) => {
            Ok("Die Antworten sind gleich und nennen weder Kennung noch Passwort.".to_string())
        }
        (false, _) => Err("Die Antworten unterscheiden sich zwischen den Versuchen.".to_string()),
        (true, Some(word)) => Err(format!("Die Antwort enthält den Hinweis „{word}“.")),
    };
    (status_result, hint_result)
}

/// S10 entry point: three attempts in total. The first two go through a separate connection, the third through
/// the wallbox itself, so the same attempt also shows whether the wallbox retries on its own after a 401.
pub async fn s10_wrong_password(ctx: &mut ScenarioCtx) {
    let snapshot = ctx.handle.snapshot();
    let was_connected = ctx.is_connected();
    // A random wrong password: a fixed one could coincide with a real one on a test system.
    let wrong = Secret::new(format!("falsch-{}", Uuid::new_v4()));
    let timeout = ctx.handle.timings().connect_timeout;
    let mut failures = Vec::new();
    for _ in 0..ATTEMPTS - 1 {
        match client::connect(
            &snapshot.config.base_url,
            &snapshot.config.identity,
            &wrong,
            timeout,
        )
        .await
        {
            Ok(socket) => {
                drop(socket);
                ctx.fail(
                    "Falsches Passwort wird abgelehnt",
                    "Die Zentrale hat die Verbindung mit falschem Passwort angenommen.",
                );
                return;
            }
            Err(failure) => failures.push(failure),
        }
    }
    let judged_ok = match attempt_through_box(ctx, wrong).await {
        Some(failure) => {
            failures.push(failure);
            true
        }
        None => false,
    };
    if judged_ok {
        let (status, hint) = judge_failures(&failures);
        record(ctx, "HTTP 401 bei jedem Versuch", status);
        record(ctx, "Kein Hinweis, welcher Teil falsch war", hint);
    }
    restore_connection(ctx, was_connected).await;
}

fn record(ctx: &mut ScenarioCtx, name: &str, outcome: Result<String, String>) {
    match outcome {
        Ok(detail) => ctx.pass(name, detail),
        Err(detail) => ctx.fail(name, detail),
    }
}

/// The third attempt: the wallbox connects with the wrong password and must stop after the 401. Returns what
/// the central system answered; `None` when the attempt could not be made or judged (a check says why).
async fn attempt_through_box(ctx: &mut ScenarioCtx, wrong: Secret) -> Option<ConnectFailure> {
    const NAME: &str = "Kein automatischer Wiederholversuch nach 401";
    if let Err(error) = ctx.handle.disconnect().await {
        ctx.fail(NAME, error.to_string());
        return None;
    }
    if let Err(error) = ctx.handle.set_password(wrong).await {
        ctx.fail(NAME, error.to_string());
        return None;
    }
    let mut rx = ctx.handle.subscribe();
    if let Err(error) = ctx.connect_box().await {
        ctx.fail(NAME, error.to_string());
        return None;
    }
    let within = ctx.tuning.connect_wait;
    let failure = match ctx
        .wait_event(&mut rx, within, |event| match event {
            BoxEvent::ConnectFailed { http_status, body } => Some(ConnectFailure {
                retryable: false,
                reason: String::new(),
                http_status: *http_status,
                body: body.clone(),
            }),
            _ => None,
        })
        .await
    {
        Wait::Ready(failure) => failure,
        Wait::TimedOut => {
            ctx.fail(
                NAME,
                "Die Wallbox hat den abgelehnten Versuch nicht gemeldet.",
            );
            return None;
        }
        Wait::Aborted => return None,
    };
    let observe = ctx.tuning.no_retry_observation;
    let further = count_attempts(ctx, &mut rx, observe).await;
    let still_failed = matches!(
        ctx.handle.snapshot().connection,
        ConnectionState::Failed { .. }
    );
    ctx.check(
        further == 0 && still_failed,
        NAME,
        format!(
            "In {} s nach dem Fehler kam kein weiterer Verbindungsversuch.",
            observe.as_secs()
        ),
        format!(
            "Die Wallbox hat {further} weitere Verbindungsversuche gestartet (oder den Fehlerzustand verlassen)."
        ),
    );
    Some(failure)
}

/// Counts connection attempts seen on `rx` during `within`. The caller has already read the events up to the
/// rejected attempt, so every attempt counted here is a retry.
async fn count_attempts(
    ctx: &mut ScenarioCtx,
    rx: &mut broadcast::Receiver<BoxEvent>,
    within: Duration,
) -> usize {
    let mut seen = 0usize;
    let until = tokio::time::Instant::now() + within;
    loop {
        let left = until.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        match ctx
            .wait_event(rx, left, |e| {
                matches!(e, BoxEvent::ConnectAttempt).then_some(())
            })
            .await
        {
            Wait::Ready(()) => seen += 1,
            Wait::TimedOut | Wait::Aborted => break,
        }
    }
    seen
}

/// Puts the wallbox back the way it was. This happens after the checks are decided, with its own time
/// allowance (the pause after the 401 can be longer than what is left of the scenario timeout), and a
/// failure is reported as a separate check that does not decide the verdict of S10.
async fn restore_connection(ctx: &mut ScenarioCtx, was_connected: bool) {
    ctx.extend_deadline_for_restore();
    let original = ctx.handle.password().clone();
    if let Err(error) = ctx.handle.set_password(original).await {
        ctx.fail(
            RESTORE_CHECK_NAME,
            format!("Das Passwort ließ sich nicht zurücksetzen: {error}"),
        );
        return;
    }
    if let Err(error) = ctx.handle.disconnect().await {
        ctx.fail(
            RESTORE_CHECK_NAME,
            format!("Die Verbindung ließ sich nicht trennen: {error}"),
        );
        return;
    }
    if was_connected {
        // After the 401 the wallbox pauses before it connects again; this waits that pause out.
        match ctx.connect_and_wait().await {
            Ok(true) | Ok(false) => {}
            Err(reason) => ctx.fail(
                RESTORE_CHECK_NAME,
                format!("Die Wallbox ist nach dem Szenario nicht wieder verbunden: {reason}"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(status: Option<u16>, body: &str) -> ConnectFailure {
        ConnectFailure {
            retryable: false,
            reason: String::new(),
            http_status: status,
            body: Some(body.to_string()),
        }
    }

    #[test]
    fn three_identical_401s_pass_both_checks() {
        let failures = vec![failure(Some(401), "Unauthorized"); 3];
        let (status, hint) = judge_failures(&failures);
        assert!(status.is_ok());
        assert!(hint.is_ok());
    }

    #[test]
    fn any_other_status_fails_the_status_check() {
        let failures = vec![
            failure(Some(401), ""),
            failure(Some(500), ""),
            failure(None, ""),
        ];
        assert!(judge_failures(&failures).0.is_err());
        assert!(judge_failures(&[]).0.is_err(), "no attempts prove nothing");
    }

    #[test]
    fn hints_and_differing_bodies_fail_the_hint_check() {
        let hinting = vec![failure(Some(401), "wrong password"); 3];
        assert!(judge_failures(&hinting).1.is_err());
        let leaking_identity = vec![failure(Some(401), "Unknown identity AP1"); 3];
        assert!(judge_failures(&leaking_identity).1.is_err());
        let differing = vec![failure(Some(401), "a"), failure(Some(401), "b")];
        assert!(judge_failures(&differing).1.is_err());
    }
}

//! S10: wrong password. Must never run against live: the backend counts failures per IP address.

use std::time::Duration;

use tokio::sync::broadcast;
use uuid::Uuid;

use super::ctx::{ScenarioCtx, Wait};
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
    let bodies: Vec<String> = failures
        .iter()
        .map(|f| f.body.clone().unwrap_or_default())
        .collect();
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

/// S10 entry point.
pub async fn s10_wrong_password(ctx: &mut ScenarioCtx) {
    let snapshot = ctx.handle.snapshot();
    let was_connected = matches!(snapshot.connection, ConnectionState::Connected { .. });
    // A random wrong password: a fixed one could coincide with a real one on a test system.
    let wrong = Secret::new(format!("falsch-{}", Uuid::new_v4()));
    let mut failures = Vec::new();
    for _ in 0..ATTEMPTS {
        match client::connect(
            &snapshot.config.base_url,
            &snapshot.config.identity,
            &wrong,
            Duration::from_secs(15),
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
    let (status, hint) = judge_failures(&failures);
    record(ctx, "HTTP 401 bei jedem Versuch", status);
    record(ctx, "Kein Hinweis, welcher Teil falsch war", hint);
    check_no_automatic_retry(ctx, wrong).await;
    restore_connection(ctx, was_connected).await;
}

fn record(ctx: &mut ScenarioCtx, name: &str, outcome: Result<String, String>) {
    match outcome {
        Ok(detail) => ctx.pass(name, detail),
        Err(detail) => ctx.fail(name, detail),
    }
}

async fn check_no_automatic_retry(ctx: &mut ScenarioCtx, wrong: Secret) {
    const NAME: &str = "Kein automatischer Wiederholversuch nach 401";
    if let Err(error) = ctx.handle.disconnect().await {
        ctx.fail(NAME, error.to_string());
        return;
    }
    if let Err(error) = ctx.handle.set_password(wrong).await {
        ctx.fail(NAME, error.to_string());
        return;
    }
    let mut rx = ctx.handle.subscribe();
    if let Err(error) = ctx.handle.connect().await {
        ctx.fail(NAME, error.to_string());
        return;
    }
    let within = ctx.tuning.connect_wait;
    let failed = ctx
        .wait_snapshot(within, |s| {
            matches!(s.connection, ConnectionState::Failed { .. })
        })
        .await;
    match failed {
        Wait::Ready(_) => {}
        Wait::TimedOut => {
            ctx.fail(NAME, "Die Box ist nach dem abgelehnten Versuch nicht in den Zustand „fehlgeschlagen“ gewechselt.");
            return;
        }
        Wait::Aborted => return,
    }
    let observe = ctx.tuning.no_retry_observation;
    let attempts = count_attempts(ctx, &mut rx, observe).await;
    let still_failed = matches!(
        ctx.handle.snapshot().connection,
        ConnectionState::Failed { .. }
    );
    ctx.check(
        attempts <= 1 && still_failed,
        NAME,
        format!("In {} s nach dem Fehler kam kein weiterer Verbindungsversuch.", observe.as_secs()),
        format!("Die Box hat {attempts} Verbindungsversuche gestartet statt einem (oder den Fehlerzustand verlassen)."),
    );
}

/// Counts connection attempts seen on `rx` during `within`, including the one that led to the failure.
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
    // The receiver was created before the connect command, so the attempt that failed is counted too:
    // anything beyond one is a retry.
    seen
}

async fn restore_connection(ctx: &mut ScenarioCtx, was_connected: bool) {
    let original = ctx.handle.password().clone();
    if let Err(error) = ctx.handle.set_password(original).await {
        ctx.fail("Passwort zurücksetzen", error.to_string());
        return;
    }
    if let Err(error) = ctx.handle.disconnect().await {
        ctx.fail("Verbindung zurücksetzen", error.to_string());
        return;
    }
    if was_connected {
        ctx.ensure_connected().await;
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

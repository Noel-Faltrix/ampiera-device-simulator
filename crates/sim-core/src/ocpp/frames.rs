//! OCPP-J frame parsing and building: `[2,id,action,payload]`, `[3,id,payload]`, `[4,id,code,description,details]`.

use serde_json::{json, Value};

const MESSAGE_TYPE_CALL: u64 = 2;
const MESSAGE_TYPE_CALL_RESULT: u64 = 3;
const MESSAGE_TYPE_CALL_ERROR: u64 = 4;

/// A parsed OCPP-J frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// Request.
    Call {
        /// Unique message id chosen by the sender.
        id: String,
        /// OCPP action name.
        action: String,
        /// Request payload.
        payload: Value,
    },
    /// Successful response.
    CallResult {
        /// Id of the call being answered.
        id: String,
        /// Response payload.
        payload: Value,
    },
    /// Error response.
    CallError {
        /// Id of the call being answered.
        id: String,
        /// OCPP error code, kept as received.
        code: String,
        /// Human-readable description from the sender.
        description: String,
        /// Extra details, `{}` when absent.
        details: Value,
    },
}

/// A frame that could not be parsed. `id` is set when the message id could still be read, so the receiver can
/// answer with a CALLERROR as OCPP-J requires.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameError {
    /// Message id if readable.
    pub id: Option<String>,
    /// What is wrong, German (shown as last error).
    pub message: String,
}

/// OCPP-J CALLERROR codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// Action is not known.
    NotImplemented,
    /// Action is known but not supported.
    NotSupported,
    /// Internal error.
    InternalError,
    /// Payload is not a valid frame.
    ProtocolError,
    /// Payload violates the schema.
    FormationViolation,
    /// Field value outside allowed range.
    PropertyConstraintViolation,
    /// Anything else.
    GenericError,
}

impl ErrorCode {
    /// The code as written on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotImplemented => "NotImplemented",
            Self::NotSupported => "NotSupported",
            Self::InternalError => "InternalError",
            Self::ProtocolError => "ProtocolError",
            Self::FormationViolation => "FormationViolation",
            Self::PropertyConstraintViolation => "PropertyConstraintViolation",
            Self::GenericError => "GenericError",
        }
    }
}

fn error(id: Option<String>, message: &str) -> FrameError {
    FrameError {
        id,
        message: message.to_string(),
    }
}

/// Parses frame text. Never panics on foreign input.
pub fn parse_frame(text: &str) -> Result<Frame, FrameError> {
    let value: Value =
        serde_json::from_str(text).map_err(|_| error(None, "Der Frame ist kein gültiges JSON."))?;
    let items = value
        .as_array()
        .ok_or_else(|| error(None, "Der Frame ist kein JSON-Array."))?;
    let message_type = items
        .first()
        .and_then(Value::as_u64)
        .ok_or_else(|| error(None, "Der Frame hat keinen Nachrichtentyp."))?;
    let id = items
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| error(None, "Der Frame hat keine Nachrichten-ID."))?
        .to_string();
    match message_type {
        MESSAGE_TYPE_CALL => parse_call(id, items),
        MESSAGE_TYPE_CALL_RESULT => {
            let payload = items
                .get(2)
                .cloned()
                .ok_or_else(|| error(Some(id.clone()), "Die Antwort enthält keine Nutzdaten."))?;
            Ok(Frame::CallResult { id, payload })
        }
        MESSAGE_TYPE_CALL_ERROR => parse_call_error(id, items),
        _ => Err(error(Some(id), "Unbekannter Nachrichtentyp.")),
    }
}

fn parse_call(id: String, items: &[Value]) -> Result<Frame, FrameError> {
    if items.len() != 4 {
        return Err(error(Some(id), "Ein CALL muss genau vier Elemente haben."));
    }
    let Some(action) = items[2].as_str() else {
        return Err(error(Some(id), "Der CALL hat keinen Aktionsnamen."));
    };
    Ok(Frame::Call {
        id,
        action: action.to_string(),
        payload: items[3].clone(),
    })
}

fn parse_call_error(id: String, items: &[Value]) -> Result<Frame, FrameError> {
    let Some(code) = items.get(2).and_then(Value::as_str) else {
        return Err(error(Some(id), "Der CALLERROR hat keinen Fehlercode."));
    };
    let description = items
        .get(3)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let details = items.get(4).cloned().unwrap_or_else(|| json!({}));
    Ok(Frame::CallError {
        id,
        code: code.to_string(),
        description,
        details,
    })
}

/// Builds a CALL frame.
pub fn build_call(id: &str, action: &str, payload: &Value) -> String {
    json!([MESSAGE_TYPE_CALL, id, action, payload]).to_string()
}

/// Builds a CALLRESULT frame.
pub fn build_call_result(id: &str, payload: &Value) -> String {
    json!([MESSAGE_TYPE_CALL_RESULT, id, payload]).to_string()
}

/// Builds a CALLERROR frame with empty details.
pub fn build_call_error(id: &str, code: ErrorCode, description: &str) -> String {
    json!([MESSAGE_TYPE_CALL_ERROR, id, code.as_str(), description, {}]).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_round_trips() {
        let text = build_call("abc", "Heartbeat", &json!({}));
        assert_eq!(text, r#"[2,"abc","Heartbeat",{}]"#);
        assert_eq!(
            parse_frame(&text).unwrap(),
            Frame::Call {
                id: "abc".into(),
                action: "Heartbeat".into(),
                payload: json!({})
            }
        );
    }

    #[test]
    fn call_result_round_trips() {
        let text = build_call_result("x1", &json!({"status": "Accepted"}));
        assert_eq!(
            parse_frame(&text).unwrap(),
            Frame::CallResult {
                id: "x1".into(),
                payload: json!({"status": "Accepted"})
            }
        );
    }

    #[test]
    fn call_error_round_trips_and_defaults_details() {
        let text = build_call_error("e1", ErrorCode::NotImplemented, "Unbekannt");
        assert_eq!(
            parse_frame(&text).unwrap(),
            Frame::CallError {
                id: "e1".into(),
                code: "NotImplemented".into(),
                description: "Unbekannt".into(),
                details: json!({})
            }
        );
        let short = parse_frame(r#"[4,"e2","GenericError"]"#).unwrap();
        assert!(
            matches!(short, Frame::CallError { ref description, .. } if description.is_empty())
        );
    }

    #[test]
    fn malformed_frames_are_errors_not_panics() {
        for text in ["", "{}", "[]", "[2]", r#"[2,5,"X",{}]"#, "nonsense"] {
            let err = parse_frame(text).unwrap_err();
            assert!(err.id.is_none(), "{text}");
        }
    }

    #[test]
    fn broken_call_keeps_the_id_for_a_callerror_answer() {
        let err = parse_frame(r#"[2,"id9","OnlyThree"]"#).unwrap_err();
        assert_eq!(err.id.as_deref(), Some("id9"));
        let err = parse_frame(r#"[2,"id9",42,{}]"#).unwrap_err();
        assert_eq!(err.id.as_deref(), Some("id9"));
        let err = parse_frame(r#"[7,"id9"]"#).unwrap_err();
        assert_eq!(err.id.as_deref(), Some("id9"));
    }

    #[test]
    fn counter_check_a_call_is_not_mistaken_for_a_result() {
        let frame = parse_frame(r#"[2,"1","BootNotification",{}]"#).unwrap();
        assert!(!matches!(frame, Frame::CallResult { .. }));
        let frame = parse_frame(r#"[3,"1",{}]"#).unwrap();
        assert!(!matches!(frame, Frame::Call { .. }));
    }
}

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Observer property IDs registered with mpv's `observe_property`.
pub const OBSERVE_TIME_POS: u64 = 1;
pub const OBSERVE_PAUSE: u64 = 2;
pub const OBSERVE_EOF_REACHED: u64 = 3;
pub const OBSERVE_DURATION: u64 = 4;

/// A command sent to mpv over the IPC socket.
#[derive(Debug, Serialize)]
pub struct MpvCommand {
    pub command: Vec<Value>,
    pub request_id: u64,
}

/// A response from mpv to a command we sent.
#[derive(Debug, Deserialize)]
pub struct MpvResponse {
    pub request_id: u64,
    pub error: String,
    pub data: Option<Value>,
}

/// A property-change event from mpv (from `observe_property`).
#[derive(Debug, Clone)]
pub struct MpvPropertyChange {
    pub id: u64,
    pub name: String,
    pub data: Option<Value>,
}

/// A raw event from mpv (seek, playback-restart, shutdown, etc.).
#[derive(Debug, Clone)]
pub struct MpvRawEvent {
    pub event: String,
}

/// Parsed message from mpv's IPC socket.
#[derive(Debug)]
pub enum MpvMessage {
    Response(MpvResponse),
    PropertyChange(MpvPropertyChange),
    Event(MpvRawEvent),
}

/// An event emitted by the IPC layer to the event translator.
#[derive(Debug, Clone)]
pub enum MpvEvent {
    PropertyChange(MpvPropertyChange),
    RawEvent(MpvRawEvent),
}

/// Parse a single line of JSON from mpv's IPC socket.
///
/// mpv sends two kinds of messages:
/// - Responses to commands (have `request_id` but no `event` key)
/// - Events (have `event` key)
///
/// We check for the `event` key to discriminate, rather than using
/// serde's untagged enum (which has poor error messages and ordering issues).
pub fn parse_message(line: &str) -> Result<MpvMessage, String> {
    let v: Value = serde_json::from_str(line).map_err(|e| format!("invalid JSON: {e}"))?;

    let obj = v.as_object().ok_or("expected JSON object")?;

    if let Some(event_val) = obj.get("event") {
        let event = event_val
            .as_str()
            .ok_or("event field is not a string")?
            .to_string();

        if event == "property-change" {
            let id = obj
                .get("id")
                .and_then(|v| v.as_u64())
                .ok_or("property-change missing id")?;
            let name = obj
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("property-change missing name")?
                .to_string();
            let data = obj.get("data").cloned();

            Ok(MpvMessage::PropertyChange(MpvPropertyChange {
                id,
                name,
                data,
            }))
        } else {
            Ok(MpvMessage::Event(MpvRawEvent { event }))
        }
    } else if obj.contains_key("request_id") {
        let resp: MpvResponse =
            serde_json::from_value(v).map_err(|e| format!("invalid response: {e}"))?;
        Ok(MpvMessage::Response(resp))
    } else {
        Err(format!("unrecognized message: {line}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_command_response_success() {
        let line = r#"{"request_id":1,"error":"success","data":null}"#;
        match parse_message(line).unwrap() {
            MpvMessage::Response(r) => {
                assert_eq!(r.request_id, 1);
                assert_eq!(r.error, "success");
                assert!(r.data.is_none() || r.data == Some(Value::Null));
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    #[test]
    fn parse_command_response_with_data() {
        let line = r#"{"request_id":5,"error":"success","data":12.345}"#;
        match parse_message(line).unwrap() {
            MpvMessage::Response(r) => {
                assert_eq!(r.request_id, 5);
                assert_eq!(r.error, "success");
                assert_eq!(r.data, Some(Value::from(12.345)));
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    #[test]
    fn parse_command_response_error() {
        let line = r#"{"request_id":3,"error":"property not found","data":null}"#;
        match parse_message(line).unwrap() {
            MpvMessage::Response(r) => {
                assert_eq!(r.request_id, 3);
                assert_eq!(r.error, "property not found");
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    #[test]
    fn parse_property_change_time_pos() {
        let line = r#"{"event":"property-change","id":1,"name":"time-pos","data":42.5}"#;
        match parse_message(line).unwrap() {
            MpvMessage::PropertyChange(p) => {
                assert_eq!(p.id, OBSERVE_TIME_POS);
                assert_eq!(p.name, "time-pos");
                assert_eq!(p.data, Some(Value::from(42.5)));
            }
            other => panic!("expected PropertyChange, got {other:?}"),
        }
    }

    #[test]
    fn parse_property_change_pause() {
        let line = r#"{"event":"property-change","id":2,"name":"pause","data":true}"#;
        match parse_message(line).unwrap() {
            MpvMessage::PropertyChange(p) => {
                assert_eq!(p.id, OBSERVE_PAUSE);
                assert_eq!(p.name, "pause");
                assert_eq!(p.data, Some(Value::Bool(true)));
            }
            other => panic!("expected PropertyChange, got {other:?}"),
        }
    }

    #[test]
    fn parse_property_change_eof_reached() {
        let line = r#"{"event":"property-change","id":3,"name":"eof-reached","data":true}"#;
        match parse_message(line).unwrap() {
            MpvMessage::PropertyChange(p) => {
                assert_eq!(p.id, OBSERVE_EOF_REACHED);
                assert_eq!(p.name, "eof-reached");
                assert_eq!(p.data, Some(Value::Bool(true)));
            }
            other => panic!("expected PropertyChange, got {other:?}"),
        }
    }

    #[test]
    fn parse_property_change_null_data() {
        // mpv sends null data when no file is loaded
        let line = r#"{"event":"property-change","id":1,"name":"time-pos","data":null}"#;
        match parse_message(line).unwrap() {
            MpvMessage::PropertyChange(p) => {
                assert_eq!(p.id, OBSERVE_TIME_POS);
                assert_eq!(p.data, Some(Value::Null));
            }
            other => panic!("expected PropertyChange, got {other:?}"),
        }
    }

    #[test]
    fn parse_seek_event() {
        let line = r#"{"event":"seek"}"#;
        match parse_message(line).unwrap() {
            MpvMessage::Event(e) => assert_eq!(e.event, "seek"),
            other => panic!("expected Event, got {other:?}"),
        }
    }

    #[test]
    fn parse_playback_restart_event() {
        let line = r#"{"event":"playback-restart"}"#;
        match parse_message(line).unwrap() {
            MpvMessage::Event(e) => assert_eq!(e.event, "playback-restart"),
            other => panic!("expected Event, got {other:?}"),
        }
    }

    #[test]
    fn parse_shutdown_event() {
        let line = r#"{"event":"shutdown"}"#;
        match parse_message(line).unwrap() {
            MpvMessage::Event(e) => assert_eq!(e.event, "shutdown"),
            other => panic!("expected Event, got {other:?}"),
        }
    }

    #[test]
    fn parse_file_loaded_event() {
        let line = r#"{"event":"file-loaded"}"#;
        match parse_message(line).unwrap() {
            MpvMessage::Event(e) => assert_eq!(e.event, "file-loaded"),
            other => panic!("expected Event, got {other:?}"),
        }
    }

    #[test]
    fn parse_invalid_json() {
        assert!(parse_message("not json").is_err());
    }

    #[test]
    fn parse_non_object() {
        assert!(parse_message("42").is_err());
    }

    #[test]
    fn parse_unknown_message() {
        // No event key and no request_id
        assert!(parse_message(r#"{"foo":"bar"}"#).is_err());
    }
}

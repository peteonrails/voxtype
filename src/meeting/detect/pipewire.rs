//! Microphone capture streams from `pw-dump` output.
//!
//! `pw-dump` prints the PipeWire graph as a JSON array of objects. Capture
//! streams are `PipeWire:Interface:Node` objects with `media.class =
//! "Stream/Input/Audio"`. Apps going through pipewire-pulse put
//! `application.name` and the process on the node; native and ALSA clients
//! may only put them on the client object, so the client's props fill in
//! whatever the node lacks.

use std::collections::HashMap;

use serde_json::Value;

use super::MicStream;

/// The open mic capture streams in a `pw-dump` document.
pub fn parse_dump(json: &str) -> Result<Vec<MicStream>, serde_json::Error> {
    let objects: Vec<Value> = serde_json::from_str(json)?;
    Ok(streams_from(&objects))
}

fn props(obj: &Value) -> Option<&serde_json::Map<String, Value>> {
    obj.get("info")?.get("props")?.as_object()
}

fn prop_str(props: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    match props.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn streams_from(objects: &[Value]) -> Vec<MicStream> {
    let clients: HashMap<u64, &serde_json::Map<String, Value>> = objects
        .iter()
        .filter(|o| o.get("type").and_then(Value::as_str) == Some("PipeWire:Interface:Client"))
        .filter_map(|o| Some((o.get("id")?.as_u64()?, props(o)?)))
        .collect();

    objects
        .iter()
        .filter(|o| o.get("type").and_then(Value::as_str) == Some("PipeWire:Interface:Node"))
        .filter(|o| {
            o.get("info")
                .and_then(|i| i.get("state"))
                .and_then(Value::as_str)
                != Some("error")
        })
        .filter_map(|o| {
            let node = props(o)?;
            if node.get("media.class").and_then(Value::as_str) != Some("Stream/Input/Audio") {
                return None;
            }
            let client = node
                .get("client.id")
                .and_then(Value::as_u64)
                .and_then(|id| clients.get(&id).copied());
            let get = |key: &str| {
                prop_str(node, key)
                    .or_else(|| client.and_then(|c| prop_str(c, key)))
                    .unwrap_or_default()
            };
            Some(MicStream {
                id: o.get("id").and_then(Value::as_u64).unwrap_or_default() as u32,
                app_name: get("application.name"),
                binary: get("application.process.binary"),
                node_name: get("node.name"),
                media_name: get("media.name"),
                pid: get("application.process.id").parse().ok(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_capture_streams_and_fills_in_from_the_client() {
        let json = r#"[
          {"id": 120, "type": "PipeWire:Interface:Client",
           "info": {"props": {"application.name": "PipeWire ALSA [hex]",
                              "application.process.binary": "hex",
                              "application.process.id": 852492}}},
          {"id": 105, "type": "PipeWire:Interface:Node",
           "info": {"state": "running",
                    "props": {"media.class": "Stream/Input/Audio",
                              "application.name": "PipeWire ALSA [hex]",
                              "node.name": "alsa_capture.hex",
                              "media.name": "ALSA Capture",
                              "client.id": 120}}},
          {"id": 70, "type": "PipeWire:Interface:Node",
           "info": {"state": "running",
                    "props": {"media.class": "Stream/Output/Audio",
                              "application.name": "Firefox"}}},
          {"id": 71, "type": "PipeWire:Interface:Node",
           "info": {"state": "error",
                    "props": {"media.class": "Stream/Input/Audio",
                              "application.name": "Broken"}}}
        ]"#;
        let streams = parse_dump(json).unwrap();
        assert_eq!(
            streams,
            vec![MicStream {
                id: 105,
                app_name: "PipeWire ALSA [hex]".into(),
                binary: "hex".into(),
                node_name: "alsa_capture.hex".into(),
                media_name: "ALSA Capture".into(),
                pid: Some(852492),
            }]
        );
    }

    #[test]
    fn rejects_documents_that_are_not_an_array() {
        assert!(parse_dump("{}").is_err());
    }
}

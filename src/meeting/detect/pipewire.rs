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

/// Which way a port carries audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

/// An audio port (`PipeWire:Interface:Port`). MIDI and control ports are
/// left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    pub id: u32,
    pub node_id: u32,
    pub direction: Direction,
    /// `audio.channel`: `MONO`, `FL`, `FR`, ...; empty when unknown
    pub channel: String,
    /// A sink's monitor port, which carries what the sink plays
    pub monitor: bool,
}

/// A link between two ports (`PipeWire:Interface:Link`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub output_port: u32,
    pub input_port: u32,
    /// `active`, `paused`, `negotiating`, `error`, ...
    pub state: String,
}

/// The parts of a `pw-dump` document the recording announcement needs: the
/// capture streams, every node's name, the audio ports and the links.
#[derive(Debug, Clone, Default)]
pub struct Graph {
    pub streams: Vec<MicStream>,
    /// `(id, node.name)` of every node
    pub nodes: Vec<(u32, String)>,
    pub ports: Vec<Port>,
    pub links: Vec<Link>,
}

impl Graph {
    /// The id of the node called `name`.
    pub fn node_named(&self, name: &str) -> Option<u32> {
        self.nodes
            .iter()
            .find(|(_, n)| n == name)
            .map(|(id, _)| *id)
    }

    /// A node's ports going `direction`, monitor ports left out.
    pub fn ports_of(&self, node_id: u32, direction: Direction) -> Vec<Port> {
        self.ports
            .iter()
            .filter(|p| p.node_id == node_id && p.direction == direction && !p.monitor)
            .cloned()
            .collect()
    }

    /// The link from `output_port` to `input_port`, if there is one.
    pub fn link(&self, output_port: u32, input_port: u32) -> Option<&Link> {
        self.links
            .iter()
            .find(|l| l.output_port == output_port && l.input_port == input_port)
    }
}

/// Parse a whole `pw-dump` document into a [`Graph`].
pub fn parse_graph(json: &str) -> Result<Graph, serde_json::Error> {
    let objects: Vec<Value> = serde_json::from_str(json)?;
    let of_type = |t: &'static str| {
        objects
            .iter()
            .filter(move |o| o.get("type").and_then(Value::as_str) == Some(t))
    };
    let id = |o: &Value| o.get("id").and_then(Value::as_u64).map(|n| n as u32);
    let nodes = of_type("PipeWire:Interface:Node")
        .filter_map(|o| {
            let name = props(o).and_then(|p| prop_str(p, "node.name"))?;
            Some((id(o)?, name))
        })
        .collect();
    let ports = of_type("PipeWire:Interface:Port")
        .filter_map(|o| {
            let info = o.get("info")?;
            let p = info.get("props")?.as_object()?;
            if !prop_str(p, "format.dsp").is_some_and(|f| f.contains("audio")) {
                return None;
            }
            let direction = match info.get("direction").and_then(Value::as_str)? {
                "input" => Direction::In,
                "output" => Direction::Out,
                _ => return None,
            };
            Some(Port {
                id: id(o)?,
                node_id: p.get("node.id").and_then(Value::as_u64)? as u32,
                direction,
                channel: prop_str(p, "audio.channel").unwrap_or_default(),
                monitor: p.get("port.monitor").and_then(Value::as_bool) == Some(true),
            })
        })
        .collect();
    let links = of_type("PipeWire:Interface:Link")
        .filter_map(|o| {
            let info = o.get("info")?;
            let port = |k: &str| info.get(k).and_then(Value::as_u64).map(|n| n as u32);
            Some(Link {
                output_port: port("output-port-id")?,
                input_port: port("input-port-id")?,
                state: info
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        })
        .collect();
    Ok(Graph {
        streams: streams_from(&objects),
        nodes,
        ports,
        links,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nodes_audio_ports_and_links() {
        let json = r#"[
          {"id": 116, "type": "PipeWire:Interface:Node",
           "info": {"state": "running",
                    "props": {"media.class": "Stream/Input/Audio",
                              "application.name": "ZOOM VoiceEngine",
                              "node.name": "ZOOM VoiceEngine"}}},
          {"id": 159, "type": "PipeWire:Interface:Port",
           "info": {"direction": "input",
                    "props": {"node.id": 116, "port.name": "input_FL",
                              "audio.channel": "FL", "format.dsp": "32 bit float mono audio"}}},
          {"id": 119, "type": "PipeWire:Interface:Port",
           "info": {"direction": "output",
                    "props": {"node.id": 116, "port.name": "monitor_FL", "audio.channel": "FL",
                              "port.monitor": true, "format.dsp": "32 bit float mono audio"}}},
          {"id": 53, "type": "PipeWire:Interface:Port",
           "info": {"direction": "input",
                    "props": {"node.id": 52, "format.dsp": "8 bit raw midi"}}},
          {"id": 172, "type": "PipeWire:Interface:Link",
           "info": {"output-node-id": 132, "output-port-id": 91,
                    "input-node-id": 116, "input-port-id": 159, "state": "active"}}
        ]"#;
        let graph = parse_graph(json).unwrap();
        assert_eq!(graph.streams.len(), 1);
        assert_eq!(graph.node_named("ZOOM VoiceEngine"), Some(116));
        assert_eq!(graph.node_named("nope"), None);
        assert_eq!(
            graph.ports_of(116, Direction::In),
            vec![Port {
                id: 159,
                node_id: 116,
                direction: Direction::In,
                channel: "FL".into(),
                monitor: false,
            }]
        );
        // The monitor port is not one of the node's outputs to link from.
        assert!(graph.ports_of(116, Direction::Out).is_empty());
        // MIDI ports are left out entirely.
        assert!(graph.ports.iter().all(|p| p.id != 53));
        assert_eq!(
            graph.link(91, 159).map(|l| l.state.as_str()),
            Some("active")
        );
        assert!(graph.link(159, 91).is_none());
    }

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

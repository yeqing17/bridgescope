//! Foreground window hierarchy inspection via `uiautomator dump`.
//!
//! The dump runs to a fixed path on the device (retried: `uiautomator` can
//! fail to reach idle on busy screens), is read back with `cat`, and is parsed
//! into a [`LayoutNode`] tree. The raw XML travels along so the UI can export
//! it unchanged.

use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bridgescope_domain::{
    BridgeError, DeviceSerial, DeviceTarget, ErrorCode, LayoutNode, LayoutSnapshot,
};
use quick_xml::events::Event;

use crate::process::{ProcessOutput, run_bounded};

const STDERR_LIMIT: usize = 16 * 1024;
const STDOUT_LIMIT: usize = 8 * 1024 * 1024;
const DUMP_REMOTE_PATH: &str = "/sdcard/bridgescope-ui.xml";
const DUMP_ATTEMPTS: usize = 3;
const RETRY_DELAY: Duration = Duration::from_millis(500);

fn adb_arguments(serial: &DeviceSerial, verb: &str, rest: &[&str]) -> Vec<String> {
    let mut arguments = vec!["-s".to_owned(), serial.as_str().to_owned(), verb.to_owned()];
    arguments.extend(rest.iter().map(|item| (*item).to_owned()));
    arguments
}

async fn adb_run(
    executable: &Path,
    serial: &DeviceSerial,
    verb: &str,
    rest: &[&str],
    timeout: Duration,
) -> Result<ProcessOutput, BridgeError> {
    let arguments: Vec<_> = adb_arguments(serial, verb, rest)
        .into_iter()
        .map(std::ffi::OsString::from)
        .collect();
    run_bounded(executable, arguments, timeout, STDOUT_LIMIT, STDERR_LIMIT).await
}

pub(crate) async fn dump_layout(
    executable: &Path,
    serial: &DeviceSerial,
    timeout: Duration,
) -> Result<LayoutSnapshot, BridgeError> {
    let mut last_error = String::new();
    for _ in 0..DUMP_ATTEMPTS {
        let output = adb_run(
            executable,
            serial,
            "shell",
            &["uiautomator", "dump", DUMP_REMOTE_PATH],
            timeout,
        )
        .await?;
        let message = String::from_utf8_lossy(&output.stdout);
        if message.contains("dumped to") {
            let read = adb_run(
                executable,
                serial,
                "exec-out",
                &["cat", DUMP_REMOTE_PATH],
                timeout,
            )
            .await?;
            let _ = adb_run(
                executable,
                serial,
                "shell",
                &["rm", "-f", DUMP_REMOTE_PATH],
                timeout,
            )
            .await;
            let xml = String::from_utf8_lossy(&read.stdout).into_owned();
            let root = parse_hierarchy(&xml)?;
            let captured_at_unix_seconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            return Ok(LayoutSnapshot {
                // Generation is a runtime concern; the runtime overwrites it
                // against its registry before emitting the event.
                target: DeviceTarget::new(serial.clone(), 1),
                root,
                raw_xml: xml,
                captured_at_unix_seconds,
            });
        }
        message.trim().clone_into(&mut last_error);
        tokio::time::sleep(RETRY_DELAY).await;
    }
    Err(BridgeError::new(
        ErrorCode::AdbFailed,
        "layout.dump_failed",
        if last_error.is_empty() {
            "uiautomator did not produce a dump".to_owned()
        } else {
            last_error
        },
    ))
}

/// Parses a `uiautomator` hierarchy document into a node tree.
///
/// Only `node` elements carry state; `hierarchy` and everything else is
/// traversed for nesting. Pre-order indices are assigned so the UI can keep a
/// stable selection key across refreshes.
pub fn parse_hierarchy(xml: &str) -> Result<LayoutNode, BridgeError> {
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut stack: Vec<LayoutNode> = Vec::new();
    let mut root: Option<LayoutNode> = None;

    let malformed =
        |detail: String| BridgeError::new(ErrorCode::Internal, "layout.parse_failed", detail);

    loop {
        let event = reader
            .read_event()
            .map_err(|error| malformed(error.to_string()))?;
        match event {
            Event::Start(element) => {
                if element.name().as_ref() == b"node" {
                    let node =
                        node_from_attributes(&element, reader.decoder()).map_err(malformed)?;
                    stack.push(node);
                }
            }
            Event::Empty(element) => {
                if element.name().as_ref() == b"node" {
                    let node =
                        node_from_attributes(&element, reader.decoder()).map_err(malformed)?;
                    attach_node(&mut stack, node, &mut root)?;
                }
            }
            Event::End(element) => {
                if element.name().as_ref() == b"node" {
                    let node = stack.pop().ok_or_else(|| {
                        malformed("hierarchy XML closes more nodes than it opens".to_owned())
                    })?;
                    attach_node(&mut stack, node, &mut root)?;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if !stack.is_empty() {
        return Err(malformed(
            "hierarchy XML ends with unclosed nodes".to_owned(),
        ));
    }
    let mut root =
        root.ok_or_else(|| malformed("no window nodes found in the hierarchy XML".to_owned()))?;
    assign_ids(&mut root, &mut 0);
    Ok(root)
}

fn attach_node(
    stack: &mut [LayoutNode],
    node: LayoutNode,
    root: &mut Option<LayoutNode>,
) -> Result<(), BridgeError> {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else if root.is_none() {
        *root = Some(node);
    } else {
        return Err(BridgeError::new(
            ErrorCode::Internal,
            "layout.parse_failed",
            "hierarchy XML has more than one root node".to_owned(),
        ));
    }
    Ok(())
}

fn assign_ids(node: &mut LayoutNode, next: &mut usize) {
    node.id = *next;
    *next += 1;
    for child in &mut node.children {
        assign_ids(child, next);
    }
}

fn node_from_attributes(
    element: &quick_xml::events::BytesStart<'_>,
    decoder: quick_xml::Decoder,
) -> Result<LayoutNode, String> {
    let mut node = LayoutNode {
        enabled: true,
        ..LayoutNode::default()
    };
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|error| error.to_string())?;
        let key = attribute.key.as_ref();
        let value = attribute
            .decode_and_unescape_value(decoder)
            .map_err(|error| error.to_string())?;
        match key {
            b"class" => node.class = value.into_owned(),
            b"resource-id" => node.resource_id = value.into_owned(),
            b"text" => node.text = value.into_owned(),
            b"content-desc" => node.content_description = value.into_owned(),
            b"bounds" => node.bounds = parse_bounds(&value)?,
            b"clickable" => node.clickable = value == "true",
            b"scrollable" => node.scrollable = value == "true",
            b"enabled" => node.enabled = value == "true",
            b"selected" => node.selected = value == "true",
            b"focused" => node.focused = value == "true",
            b"package" => node.package = value.into_owned(),
            _ => {}
        }
    }
    Ok(node)
}

/// Parses `[x1,y1][x2,y2]` into `[x, y, width, height]`.
fn parse_bounds(value: &str) -> Result<[i32; 4], String> {
    let digits: Vec<&str> = value
        .split([',', ']', '['])
        .filter(|part| !part.is_empty())
        .collect();
    if digits.len() != 4 {
        return Err(format!("unsupported bounds {value:?}"));
    }
    let mut numbers = [0_i32; 4];
    for (slot, part) in numbers.iter_mut().zip(digits.iter()) {
        *slot = part.parse::<i32>().map_err(|error| error.to_string())?;
    }
    let [x1, y1, x2, y2] = numbers;
    Ok([x1, y1, x2.saturating_sub(x1), y2.saturating_sub(y1)])
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy rotation="0">
  <node index="0" text="" resource-id="" class="android.widget.FrameLayout"
        package="com.example" content-desc="" checkable="false" checked="false"
        clickable="false" enabled="true" focusable="false" focused="false"
        scrollable="false" long-clickable="false" password="false" selected="false"
        bounds="[0,0][1080,2400]">
    <node index="1" text="Hello" resource-id="com.example:id/title"
          class="android.widget.TextView" package="com.example" content-desc="greet"
          clickable="true" enabled="true" focused="true" scrollable="true"
          bounds="[24,96][540,192]" />
    <node index="2" text="" resource-id="" class="android.widget.Button"
          package="com.example" content-desc="" clickable="true" enabled="false"
          bounds="[24,200][1056,296]" />
  </node>
</hierarchy>"#;

    #[test]
    fn parses_sample_hierarchy_into_a_tree() {
        let root = parse_hierarchy(SAMPLE).expect("parses");
        assert_eq!(root.class, "android.widget.FrameLayout");
        assert_eq!(root.bounds, [0, 0, 1080, 2400]);
        assert_eq!(root.children.len(), 2);
        let title = &root.children[0];
        assert_eq!(title.text, "Hello");
        assert_eq!(title.resource_id, "com.example:id/title");
        assert_eq!(title.content_description, "greet");
        assert!(title.clickable && title.focused && title.scrollable);
        assert_eq!(title.bounds, [24, 96, 516, 96]);
        assert!(!root.children[1].enabled);
        assert_eq!(root.count(), 3);
        // Pre-order ids are stable selection keys.
        assert_eq!(root.id, 0);
        assert_eq!(root.children[0].id, 1);
        assert_eq!(root.children[1].id, 2);
    }

    #[test]
    fn rejects_malformed_documents() {
        assert!(parse_hierarchy("not xml at all").is_err());
        assert!(parse_hierarchy("<hierarchy></hierarchy>").is_err());
    }

    #[test]
    fn bounds_parser_reports_width_and_height() {
        assert_eq!(parse_bounds("[10,20][30,60]"), Ok([10, 20, 20, 40]));
        assert!(parse_bounds("junk").is_err());
    }
}

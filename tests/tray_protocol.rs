#![cfg(target_os = "linux")]
use futures_util::StreamExt;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, watch};
use voxtype::tray::{self, TrayAction, TrayState};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

#[derive(Clone, Default)]
struct Watcher {
    items: Arc<Mutex<Vec<String>>>,
}
#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(&self, service: &str) {
        self.items.lock().unwrap().push(service.to_owned());
    }
    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().unwrap().clone()
    }
}
async fn watcher(w: Watcher) -> zbus::Connection {
    zbus::connection::Builder::session()
        .unwrap()
        .name("org.kde.StatusNotifierWatcher")
        .unwrap()
        .serve_at("/StatusNotifierWatcher", w)
        .unwrap()
        .build()
        .await
        .unwrap()
}
async fn registered(w: &Watcher) -> String {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(name) = w.items.lock().unwrap().first() {
                return name.clone();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("tray must register when a host starts after the daemon")
}
type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);
type Pixmaps = Vec<(i32, i32, Vec<u8>)>;
type Tooltip = (String, Pixmaps, String, String);
async fn menu(p: &zbus::Proxy<'_>) -> Vec<Layout> {
    let (_, layout): (u32, Layout) = p
        .call("GetLayout", &(0_i32, -1_i32, Vec::<String>::new()))
        .await
        .unwrap();
    layout
        .2
        .into_iter()
        .map(|child| child.try_into().unwrap())
        .collect()
}
fn item<'a>(menu: &'a [Layout], label: &str) -> &'a Layout {
    menu.iter()
        .find(|(_, props, _)| {
            props.get("label").and_then(|v| <&str>::try_from(v).ok()) == Some(label)
        })
        .unwrap()
}
async fn click(p: &zbus::Proxy<'_>, id: i32) {
    p.call::<_, _, ()>("Event", &(id, "clicked", Value::from(0_i32), 0_u32))
        .await
        .unwrap();
}
fn isolated(name: &str) -> bool {
    if std::env::var_os("VOXTYPE_TRAY_ISOLATED_TEST").is_some() {
        return false;
    }
    let status = std::process::Command::new("dbus-run-session")
        // Nix's build sandbox has no system /etc/dbus-1/session.conf.
        .arg("--config-file")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/tray-session.conf"
        ))
        .arg("--")
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("VOXTYPE_TRAY_ISOLATED_TEST", "1")
        .status()
        .unwrap();
    assert!(status.success());
    true
}

#[tokio::test]
async fn tray_waits_for_late_watcher() {
    if isolated("tray_waits_for_late_watcher") {
        return;
    }
    assert!(tray::TrayRuntime::start(false).is_none());
    let (tx, rx) = watch::channel(TrayState::Idle);
    let (actions, mut events) = mpsc::channel(8);
    let task = tray::spawn_with_actions(rx, actions);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let w = Watcher::default();
    let bus = watcher(w.clone()).await;
    let name = registered(&w).await;
    // SNI uses NewStatus/NewIcon rather than the standard PropertiesChanged
    // signal for these fields. A generic zbus property cache would be stale.
    let sni: zbus::Proxy<'_> = zbus::proxy::Builder::new(&bus)
        .destination(name.as_str())
        .unwrap()
        .path("/StatusNotifierItem")
        .unwrap()
        .interface("org.kde.StatusNotifierItem")
        .unwrap()
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .unwrap();
    assert_eq!(sni.get_property::<String>("Id").await.unwrap(), "voxtype");
    assert!(
        sni.get_property::<bool>("ItemIsMenu").await.unwrap(),
        "left click must open the host menu, not settings"
    );
    assert_eq!(
        sni.get_property::<String>("Status").await.unwrap(),
        "Active"
    );
    let pixels: Vec<(i32, i32, Vec<u8>)> = sni.get_property("IconPixmap").await.unwrap();
    assert!(!pixels.is_empty());
    assert_eq!(pixels[0].2.len(), (pixels[0].0 * pixels[0].1 * 4) as usize);
    let path: OwnedObjectPath = sni.get_property("Menu").await.unwrap();
    let dbusmenu = zbus::Proxy::new(&bus, name.as_str(), path.as_str(), "com.canonical.dbusmenu")
        .await
        .unwrap();
    let idle = menu(&dbusmenu).await;
    assert!(!bool::try_from(&item(&idle, "Cancel").1["enabled"]).unwrap());
    let activation = sni.call::<_, _, ()>("Activate", &(0_i32, 0_i32)).await;
    assert!(activation.unwrap_err().to_string().contains("ItemIsMenu"));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), events.recv())
            .await
            .is_err(),
        "plain activation must not launch settings"
    );
    click(&dbusmenu, item(&idle, "Settings").0).await;
    assert_eq!(events.recv().await, Some(TrayAction::OpenSettings));
    click(&dbusmenu, item(&idle, "Start Recording").0).await;
    assert_eq!(events.recv().await, Some(TrayAction::ToggleRecording));
    let mut signals = sni.receive_signal("NewIcon").await.unwrap();
    tx.send(TrayState::Recording).unwrap();
    tokio::time::timeout(Duration::from_secs(2), signals.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        sni.get_property::<String>("Status").await.unwrap(),
        "NeedsAttention"
    );
    assert_eq!(sni.get_property::<String>("IconName").await.unwrap(), "");
    let recording = menu(&dbusmenu).await;
    click(&dbusmenu, item(&recording, "Stop Recording").0).await;
    assert_eq!(events.recv().await, Some(TrayAction::ToggleRecording));
    assert!(item(&recording, "Cancel")
        .1
        .get("enabled")
        .map(|value| bool::try_from(value).unwrap())
        .unwrap_or(true));
    click(&dbusmenu, item(&recording, "Cancel").0).await;
    assert_eq!(events.recv().await, Some(TrayAction::Cancel));
    tx.send(TrayState::Transcribing).unwrap();
    tokio::time::timeout(Duration::from_secs(2), signals.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sni.get_property::<String>("IconName").await.unwrap(), "");
    let busy = menu(&dbusmenu).await;
    assert!(!bool::try_from(&item(&busy, "Start Recording").1["enabled"]).unwrap());
    click(&dbusmenu, item(&busy, "Start Recording").0).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), events.recv())
            .await
            .is_err()
    );
    tx.send(TrayState::Error).unwrap();
    tokio::time::timeout(Duration::from_secs(2), signals.next())
        .await
        .unwrap()
        .unwrap();
    let tooltip: Tooltip = sni.get_property("ToolTip").await.unwrap();
    assert!(tooltip.3.contains("failed"));
    assert!(
        tooltip.2.contains("failed"),
        "compact hosts display the title only"
    );
    assert_eq!(
        w.items.lock().unwrap().len(),
        1,
        "updates must not re-register"
    );
    bus.release_name("org.kde.StatusNotifierWatcher")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let restarted = Watcher::default();
    let _restarted_bus = watcher(restarted.clone()).await;
    assert_eq!(registered(&restarted).await, name);
    assert_eq!(restarted.items.lock().unwrap().len(), 1);
    drop(tx);
    task.await.unwrap();
    let dbus = zbus::fdo::DBusProxy::new(&bus).await.unwrap();
    assert!(
        !dbus
            .name_has_owner(name.as_str().try_into().unwrap())
            .await
            .unwrap(),
        "tray service must be removed on channel close"
    );
    println!("SNI_PROTOCOL_PASS: late host, active idle, pixmap, menu events, state signals, error, one registration, watcher restart, exit cleanup");
}

#[tokio::test]
async fn language_radio_event_reaches_daemon_channel_without_optimistic_check() {
    if isolated("language_radio_event_reaches_daemon_channel_without_optimistic_check") {
        return;
    }
    let w = Watcher::default();
    let bus = watcher(w.clone()).await;
    let mut config = voxtype::config::Config::default();
    config.set_model_for(
        voxtype::config::TranscriptionEngine::Cohere,
        "cohere-transcribe-03-2026-Q4_K_M.gguf",
    );
    let mut runtime = tray::TrayRuntime::start_for_config(&config).unwrap();
    let name = registered(&w).await;
    let sni = zbus::Proxy::new(
        &bus,
        name.as_str(),
        "/StatusNotifierItem",
        "org.kde.StatusNotifierItem",
    )
    .await
    .unwrap();
    assert!(sni.get_property::<bool>("ItemIsMenu").await.unwrap());
    let path: OwnedObjectPath = sni.get_property("Menu").await.unwrap();
    let proxy = zbus::Proxy::new(&bus, name.as_str(), path.as_str(), "com.canonical.dbusmenu")
        .await
        .unwrap();
    let root = menu(&proxy).await;
    let nested: Vec<Layout> = item(&root, "Language: en")
        .2
        .iter()
        .map(|v| v.try_clone().unwrap().try_into().unwrap())
        .collect();
    let french = item(&nested, "french (fr)");
    assert_eq!(i32::try_from(&french.1["toggle-state"]).unwrap(), 0);
    assert_eq!(<&str>::try_from(&french.1["toggle-type"]).unwrap(), "radio");
    click(&proxy, french.0).await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), runtime.next_language())
            .await
            .unwrap(),
        Some("fr")
    );
    let root = menu(&proxy).await;
    assert!(!item(&root, "Language: en").2.is_empty());
    config.cohere.as_mut().unwrap().language = "fr".into();
    runtime.update_language(voxtype::language_selection::LanguageMenu::for_config(
        &config,
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    let root = menu(&proxy).await;
    let nested: Vec<Layout> = item(&root, "Language: fr")
        .2
        .iter()
        .map(|v| v.try_clone().unwrap().try_into().unwrap())
        .collect();
    assert_eq!(
        i32::try_from(&item(&nested, "french (fr)").1["toggle-state"]).unwrap(),
        1
    );
    assert_eq!(
        nested
            .iter()
            .filter(|v| v.1.contains_key("toggle-type"))
            .count(),
        14
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn missing_bus_is_nonfatal() {
    if std::env::var_os("VOXTYPE_TRAY_MISSING_BUS_TEST").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "missing_bus_is_nonfatal"])
            .env("VOXTYPE_TRAY_MISSING_BUS_TEST", "1")
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/definitely-missing-voxtype-test-bus",
            )
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let (_tx, rx) = watch::channel(TrayState::Idle);
    let task = tray::spawn(rx);
    tokio::time::timeout(Duration::from_secs(6), task)
        .await
        .unwrap()
        .unwrap();
}

//! Recording-free probe for protocol/desktop integration. No daemon or audio access.
#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    use voxtype::tray::{spawn_with_actions, TrayState};
    if let Some(path) = std::env::args().nth(1) {
        let path = std::path::PathBuf::from(path);
        let mut config: voxtype::config::Config =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let (actions, mut events) = tokio::sync::mpsc::channel(8);
        let mut runtime =
            voxtype::tray::TrayRuntime::start_for_config_with_actions(&config, actions).unwrap();
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        println!("LANGUAGE_PROBE (no audio or daemon; config fixture only)");
        loop {
            tokio::select! {
                line = lines.next_line() => match line {
                    Ok(Some(line)) if line == "quit" => break,
                    Ok(Some(line)) => runtime.update(TrayState::from_state_name(&line)),
                    _ => break,
                },
                Some(language) = runtime.next_language() => {
                    match voxtype::language_selection::persist(&config, &path, language) {
                        Ok(next) => {
                            config = next;
                            runtime.update_language(voxtype::language_selection::LanguageMenu::for_config(&config));
                            println!("LANGUAGE_SAVED {language}");
                        }
                        Err(error) => println!("LANGUAGE_FAILED {error}"),
                    }
                }
                Some(action) = events.recv() => println!("ACTION {action:?} (not executed)"),
            }
        }
        runtime.shutdown().await;
        return;
    }
    let (tx, rx) = tokio::sync::watch::channel(TrayState::Idle);
    let (actions, mut events) = tokio::sync::mpsc::channel(8);
    let task = spawn_with_actions(rx, actions);
    println!(
        "TRAY_PROBE pid={} (no recording actions execute)",
        std::process::id()
    );
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(line)) if line == "quit" => break,
                Ok(Some(line)) => { let state = TrayState::from_state_name(&line); tx.send_replace(state); println!("STATE {state:?}"); },
                _ => break,
            },
            Some(action) = events.recv() => println!("ACTION {action:?} (not executed)"),
        }
    }
    drop(tx);
    task.await.unwrap();
    println!("TRAY_PROBE_EXIT");
}
#[cfg(not(target_os = "linux"))]
fn main() {}

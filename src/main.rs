fn main() {
    let signal_manager = signal_manager::SignalManager::new(
        "/home/jerin/RustProjects/signal-messenger-handler/messages/".to_string(),
    );
    loop {
        signal_manager.send_messages();
        signal_manager.receive_messages();
    }
}

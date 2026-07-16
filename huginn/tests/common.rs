use huginn_core::event::EventHub;
use std::sync::Arc;

pub async fn start_server(port: u16) -> Arc<EventHub> {
    let hub = Arc::new(EventHub::new(256));
    let hub_clone = Arc::clone(&hub);
    tokio::spawn(async move {
        huginn_web::server::run_server(port, hub_clone).await.ok();
    });
    hub
}

pub fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

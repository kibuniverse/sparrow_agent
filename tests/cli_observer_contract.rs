use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use sparrow_agent::cli_observer::{
    browser_task_url, default_frontend_dist, inspect_addr_from_env_value, replay_trace_url,
};

#[test]
fn browser_task_url_points_to_task_route() {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8787);

    assert_eq!(
        browser_task_url(addr, "task_abc"),
        "http://127.0.0.1:8787/tasks/task_abc"
    );
}

#[test]
fn replay_trace_url_points_to_replay_route() {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8787);

    assert_eq!(
        replay_trace_url(addr, "task_abc.sparrow-trace.json"),
        "http://127.0.0.1:8787/replay/task_abc.sparrow-trace.json"
    );
}

#[test]
fn inspect_addr_from_env_value_uses_default_when_empty() {
    assert_eq!(
        inspect_addr_from_env_value(None).unwrap().to_string(),
        "127.0.0.1:8787"
    );
    assert_eq!(
        inspect_addr_from_env_value(Some("")).unwrap().to_string(),
        "127.0.0.1:8787"
    );
}

#[test]
fn inspect_addr_from_env_value_parses_override() {
    assert_eq!(
        inspect_addr_from_env_value(Some("127.0.0.1:9797"))
            .unwrap()
            .to_string(),
        "127.0.0.1:9797"
    );
}

#[test]
fn default_frontend_dist_points_at_frontend_dist() {
    assert!(default_frontend_dist().ends_with("frontend/dist"));
}

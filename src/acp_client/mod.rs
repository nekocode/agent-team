pub mod team_client;

// 编译期断言：ClientSideConnection 必须 Send（无需 #[test]，编译即检查）
fn _assert_send() {
    fn check<T: Send>() {}
    check::<agent_client_protocol::ClientSideConnection>();
}

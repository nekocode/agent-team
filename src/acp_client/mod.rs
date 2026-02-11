pub mod team_client;

#[cfg(test)]
mod tests {
    #[test]
    fn client_side_connection_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<agent_client_protocol::ClientSideConnection>();
    }
}

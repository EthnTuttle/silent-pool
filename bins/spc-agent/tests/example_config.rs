#[test]
fn example_config_is_valid() {
    let text = include_str!("../../../config/spc-agent.example.toml");
    let cfg: spc_agent::config::AgentConfig = toml::from_str(text).unwrap();
    cfg.validate().unwrap();
}

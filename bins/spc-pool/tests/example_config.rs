#[test]
fn example_config_is_valid() {
    let text = include_str!("../../../config/spc-pool.example.toml");
    let cfg: spc_pool::config::Config = toml::from_str(text).unwrap();
    cfg.validate().unwrap();
}

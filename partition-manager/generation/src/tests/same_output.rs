#[test]
fn same_output() {
    let json_text = include_str!("partitions.json");
    let json_output = crate::transform_json_manifest(json_text).unwrap();

    let yaml_text = include_str!("partitions.yaml");
    let yaml_output = crate::transform_yaml_manifest(yaml_text).unwrap();

    let toml_text = include_str!("partitions.toml");
    let toml_output = crate::transform_toml_manifest(toml_text).unwrap();

    assert_eq!(json_output, yaml_output);
    assert_eq!(json_output, toml_output);
}

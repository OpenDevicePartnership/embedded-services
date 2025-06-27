extern crate std;

use std::format;

#[test]
fn duplicate() {
    let toml = "[partitions]
    test = {\"offset\" = 1, \"size\" = 1}
    test = {\"offset\" = 1, \"size\" = 1}";
    let output = crate::transform_toml_manifest(toml);

    assert_eq!(
        format!("{:?}", output),
        "Err(TOML parse error at line 3, column 5\n  |\n3 |     test = {\"offset\" = 1, \"size\" = 1}\n  |     ^\nduplicate key `test` in table `partitions`\n)"
    );
}

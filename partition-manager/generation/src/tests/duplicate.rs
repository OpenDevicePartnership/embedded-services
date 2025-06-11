extern crate std;

use std::format;

#[test]
fn duplicate() {
    let json = "{\"disk\": {}, \"partitions\": {\"test\": {\"offset\": 1, \"size\": 1}, \"test\": {\"offset\": 2, \"size\": 1}}}";
    let output = crate::transform_json_manifest(json);

    assert_eq!(
        format!("{:?}", output),
        "Err(Duplicate key test in partitions at line 1 column 95)"
    );
}

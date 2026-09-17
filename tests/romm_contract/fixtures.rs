#[test]
fn canonical_sipario_plan_fixtures_are_self_consistent() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/sipario-download-plans.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 5);
    for case in cases {
        let plan = &case["plan"];
        let file_ids: std::collections::BTreeSet<i64> = plan["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file["id"].as_i64().unwrap())
            .collect();
        assert_eq!(file_ids.len(), plan["files"].as_array().unwrap().len());
        let preferred = plan["preferred_file_id"].as_i64().unwrap();
        assert!(file_ids.contains(&preferred));
        assert!(
            plan["files"]
                .as_array()
                .unwrap()
                .iter()
                .find(|file| file["id"] == preferred)
                .unwrap()["launchable"]
                .as_bool()
                .unwrap()
        );
        for dependency in plan["dependencies"].as_array().unwrap() {
            assert!(file_ids.contains(&dependency["parent_file_id"].as_i64().unwrap()));
            assert!(file_ids.contains(&dependency["child_file_id"].as_i64().unwrap()));
        }
        for file in plan["files"].as_array().unwrap() {
            let id = file["id"].as_i64().unwrap().to_string();
            let bytes = general_purpose::STANDARD
                .decode(case["file_bytes_base64"][&id].as_str().unwrap())
                .unwrap();
            assert_eq!(
                bytes.len() as i64,
                file["file_size_bytes"].as_i64().unwrap()
            );
        }
        assert_eq!(
            case["expected_closure"].as_array().unwrap()[0],
            plan["preferred_file_id"]
        );
    }
}

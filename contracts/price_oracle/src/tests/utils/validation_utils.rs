#[cfg(test)]
mod unit {
    use crate::model::error::illegal_argument;
    use crate::utils::validate_name_uniqueness;
    use result_extensions::ResultExtensions;

    #[test]
    fn validate_names_are_unique() {
        validate_name_uniqueness(&vec![
            "pomme".to_string(),
            "poire".to_string(),
            "banane".to_string(),
        ])
        .unwrap();
    }

    #[test]
    fn validate_duplicate_names_are_detected() {
        let result = validate_name_uniqueness(&vec![
            "pomme".to_string(),
            "poire".to_string(),
            "banane".to_string(),
            "pomme".to_string(),
        ]);
        assert_eq!(result, illegal_argument("Duplicate name: pomme").to_err());
    }
}

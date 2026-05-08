use schemars::gen::SchemaGenerator;
use schemars::schema::{InstanceType, ObjectValidation, Schema, SchemaObject};

pub fn map_additional_true(_gen: &mut SchemaGenerator) -> Schema {
    let obj_validation = ObjectValidation {
        additional_properties: Some(Box::new(Schema::Bool(true))),
        ..Default::default()
    };

    let schema_obj = SchemaObject {
        instance_type: Some(InstanceType::Object.into()),
        object: Some(Box::new(obj_validation)),
        ..Default::default()
    };

    Schema::Object(schema_obj)
}

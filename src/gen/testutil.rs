#![cfg(test)]

use crate::ir::{
    ApiSpec, AuthMethod, EnumVariant, ErrorResponse, FieldDef, HttpMethod, OpParameter,
    OpRequestBody, Operation, ParamLocation, RustType, TypeDef,
};
use heck::{ToSnakeCase, ToUpperCamelCase};

#[must_use]
pub fn make_field(name: &str, rust_type: RustType, required: bool) -> FieldDef {
    FieldDef {
        name: name.into(),
        rust_name: name.to_snake_case(),
        rust_type,
        required,
        description: None,
        default_value: None,
    }
}

#[must_use]
pub fn make_struct(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.into(),
        rust_name: name.to_upper_camel_case(),
        fields,
        is_enum: false,
        enum_variants: Vec::new(),
        description: None,
    }
}

#[must_use]
pub fn make_enum(name: &str, variants: Vec<&str>) -> TypeDef {
    TypeDef {
        name: name.into(),
        rust_name: name.to_upper_camel_case(),
        fields: Vec::new(),
        is_enum: true,
        enum_variants: variants
            .into_iter()
            .map(|v| EnumVariant {
                name: v.into(),
                rust_name: v.to_upper_camel_case(),
            })
            .collect(),
        description: None,
    }
}

#[must_use]
pub fn make_spec_with(types: Vec<TypeDef>, operations: Vec<Operation>) -> ApiSpec {
    ApiSpec {
        name: "TestApi".into(),
        description: None,
        version: "1.0.0".into(),
        base_url: None,
        auth: AuthMethod::None,
        operations,
        types,
    }
}

#[must_use]
pub fn make_get_op(id: &str, response_type: RustType) -> Operation {
    Operation {
        id: id.into(),
        method: HttpMethod::Get,
        path: format!("/{id}"),
        summary: Some(format!("Get {id}")),
        description: None,
        parameters: vec![],
        request_body: None,
        response_type: Some(response_type),
        errors: vec![],
    }
}

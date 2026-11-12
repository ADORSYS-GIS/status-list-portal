//! `OpenAPI` description of the backend's API.
//!
//! The document is generated from the `#[utoipa::path]` attributes of the handlers. A
//! snapshot test keeps the committed copy in `src/snapshots/` up to date. Another test checks
//! that the `operationId`s, which utoipa takes from the handlers' function names unless
//! `operation_id` overrides them, are unique.

use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

#[derive(OpenApi)]
#[openapi(info(
    title = "Status List Portal API",
    description = "API between the portal's frontend and its backend. It is not a public \
                   contract and can change in any release.",
    version = "0.0.0",
))]
pub struct BaseOpenApi;

impl BaseOpenApi {
    pub fn router<S: Send + Sync + Clone + 'static>() -> OpenApiRouter<S> {
        OpenApiRouter::with_openapi(Self::openapi())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{Map, Value};

    /// Fields of a Path Item Object that are not operations. Every other field is one, so an
    /// operation a later `OpenAPI` version or utoipa release adds is still checked.
    const NON_OPERATION_FIELDS: [&str; 6] = [
        "$ref",
        "summary",
        "description",
        "servers",
        "parameters",
        "additionalOperations",
    ];

    #[test]
    fn openapi_snapshot() {
        let openapi = crate::router::openapi_router().into_openapi();
        insta::assert_json_snapshot!(openapi);
    }

    /// Only `paths` is scanned: the API has no webhooks or callbacks, whose operations would
    /// also need unique IDs.
    #[test]
    fn operation_ids_are_unique() {
        let openapi = crate::router::openapi_router().into_openapi();
        let document = serde_json::to_value(&openapi).expect("the document serializes");
        let paths = document["paths"]
            .as_object()
            .expect("the document has paths");

        let mut operations_by_id: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for (path, item) in paths {
            let item = item.as_object().expect("a path item is an object");
            let operations = item
                .iter()
                .filter(|(field, _)| {
                    !NON_OPERATION_FIELDS.contains(&field.as_str()) && !field.starts_with("x-")
                })
                .chain(
                    item.get("additionalOperations")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flat_map(Map::iter),
                );
            for (method, operation) in operations {
                let method = method.to_uppercase();
                let id = operation["operationId"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{method} {path} has no operationId"));
                operations_by_id
                    .entry(id)
                    .or_default()
                    .push(format!("{method} {path}"));
            }
        }

        let clashes: BTreeMap<_, _> = operations_by_id
            .into_iter()
            .filter(|(_, operations)| operations.len() > 1)
            .collect();
        assert!(
            clashes.is_empty(),
            "operation IDs must be unique, but some are shared, maybe by handlers with the same \
             name in different modules; rename one of the handlers or set `operation_id` in its \
             `#[utoipa::path]`: {clashes:?}"
        );
    }
}

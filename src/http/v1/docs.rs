use std::sync::Arc;

use axum::{
    Router,
    http::StatusCode,
    response::Response,
    routing::{MethodFilter, on},
};
use serde_json::{Value, json};
use utoipa::OpenApi;
use utoipa_swagger_ui::{Config, SwaggerUi};

use crate::{
    http::{
        codec::{JsonResponseFormat, json_success_response},
        extract::NoQuery,
        health::__path_health_handler,
        v1::{
            github::{
                __path_get_github_owner_handler, __path_get_github_repo_handler,
                __path_get_github_repo_languages_handler, __path_list_github_owner_repos_handler,
                __path_list_github_repo_activity_handler, __path_list_github_repo_tags_handler,
            },
            hello::{__path_create_hello_handler, __path_get_hello_handler},
            items::__path_list_items_handler,
            profile::{
                __path_create_profile_handler, __path_delete_profile_handler,
                __path_get_profile_handler, __path_update_profile_handler,
                DependencyUnavailableProblemResponse, UnauthorizedProblemResponse,
            },
        },
    },
    problem::{ProblemCode, ProblemDetails, ProblemIssue, ProblemResponse, ProblemSource},
    state::AppState,
    validation::SAFE_INTEGER_MAX,
};

const TIMESTAMP_PATTERN: &str = r"^[0-9]{4}-(?:0[1-9]|1[0-2])-(?:0[1-9]|[12][0-9]|3[01])T(?:[01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9]\.[0-9]{3}Z$";
const BOUNDED_NAME_PATTERN: &str = r"^(?![\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000])(?!.*[\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]$)[^\u0000-\u001F\u007F-\u009F]*$";
const CONTACT_EMAIL_PATTERN: &str = r"^(?=.{1,64}@)(?!\.)(?![^@]*\.\.)(?![^@]*\.@)[A-Za-z0-9!#$%&'*+/=?^_`{|}~.-]+@[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$";
const CONTACT_EMAIL_INPUT_PATTERN: &str = r"^[\t-\r ]*(?=[^\t-\r ]{1,254}[\t-\r ]*$)(?!\.)(?![^@]*\.\.)(?![^@]*\.@)[A-Za-z0-9!#$%&'*+/=?^_`{|}~.-]{1,64}@[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?)+[\t-\r ]*$";
const PHONE_NUMBER_PATTERN: &str = r"^\+[1-9][0-9]{6,14}$";
const PHONE_NUMBER_INPUT_PATTERN: &str = r"^[\t-\r ]*\+[1-9][0-9]{6,14}[\t-\r ]*$";

#[derive(OpenApi)]
#[openapi(
    info(title = "Axum Portable API", version = "1.0.0"),
    paths(
        health_handler,
        get_hello_handler,
        create_hello_handler,
        list_items_handler,
        create_profile_handler,
        get_profile_handler,
        update_profile_handler,
        delete_profile_handler,
        get_github_owner_handler,
        list_github_owner_repos_handler,
        get_github_repo_handler,
        list_github_repo_activity_handler,
        get_github_repo_languages_handler,
        list_github_repo_tags_handler
    ),
    components(
        schemas(ProblemCode, ProblemDetails, ProblemIssue, ProblemSource),
        responses(
            ProblemResponse,
            UnauthorizedProblemResponse,
            DependencyUnavailableProblemResponse
        )
    )
)]
struct ApiDoc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/openapi.json", on(MethodFilter::GET, openapi_handler))
}

pub fn ui_router() -> Router<Arc<AppState>> {
    Router::new().merge(
        SwaggerUi::new("/api-docs").config(Config::from("/openapi.json").validator_url("none")),
    )
}

async fn openapi_handler(format: JsonResponseFormat, _query: NoQuery) -> Response {
    json_success_response(StatusCode::OK, format, &openapi_document())
}

#[must_use]
pub fn openapi_document() -> Value {
    let mut document =
        serde_json::to_value(ApiDoc::openapi()).expect("generated OpenAPI should serialize");
    document["openapi"] = json!("3.1.0");
    document["jsonSchemaDialect"] = json!("https://spec.openapis.org/oas/3.1/dialect/base");
    document["components"]["securitySchemes"]["bearerAuth"] = json!({
        "type": "http",
        "scheme": "bearer",
        "description": "Firebase ID token for the current principal"
    });

    inline_component_responses(&mut document);
    strengthen_known_schemas(&mut document);
    strengthen_operations(&mut document);
    document
}

fn inline_component_responses(document: &mut Value) {
    let components = document["components"]["responses"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let Some(paths) = document["paths"].as_object_mut() else {
        return;
    };
    for path in paths.values_mut() {
        let Some(methods) = path.as_object_mut() else {
            continue;
        };
        for operation in methods.values_mut() {
            let Some(responses) = operation
                .get_mut("responses")
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            for response in responses.values_mut() {
                let Some(reference) = response.get("$ref").and_then(Value::as_str) else {
                    continue;
                };
                let Some(name) = reference.strip_prefix("#/components/responses/") else {
                    continue;
                };
                if let Some(component) = components.get(name) {
                    *response = component.clone();
                }
            }
        }
    }
}

fn strengthen_known_schemas(document: &mut Value) {
    let schemas = &mut document["components"]["schemas"];
    set_property(
        schemas,
        "Health",
        "status",
        json!({"type":"string", "const":"healthy"}),
    );
    set_property(
        schemas,
        "Money",
        "currency",
        json!({"type":"string", "const":"USD"}),
    );
    for schema in ["ProfileCreate", "Profile"] {
        set_property(
            schemas,
            schema,
            "termsAccepted",
            json!({"type":"boolean", "const":true}),
        );
    }
    set_property(
        schemas,
        "Item",
        "category",
        json!({"type":"string", "enum":["electronics","tools","accessories","robotics","power","components"]}),
    );
    strengthen_portable_scalars(schemas);
    strengthen_profile_schemas(schemas);
    strengthen_item_schemas(schemas);
    strengthen_github_schemas(schemas);
    strengthen_problem_schemas(schemas);
}

fn strengthen_portable_scalars(schemas: &mut Value) {
    set_property(schemas, "HelloCreate", "name", bounded_name_schema());
    for schema in ["Profile", "ProfileCreate", "ProfileUpdate"] {
        set_property(schemas, schema, "firstName", bounded_name_schema());
        set_property(schemas, schema, "lastName", bounded_name_schema());
    }
    set_property(schemas, "Item", "createdAt", timestamp_schema());
    for property in ["createdAt", "updatedAt"] {
        set_property(schemas, "Profile", property, timestamp_schema());
    }
    set_property(schemas, "Profile", "id", opaque_id_schema());
}

fn strengthen_profile_schemas(schemas: &mut Value) {
    for schema in ["ProfileCreate", "ProfileUpdate"] {
        set_property(
            schemas,
            schema,
            "contactEmail",
            json!({
                "type": "string",
                "pattern": CONTACT_EMAIL_INPUT_PATTERN,
                "description": "ASCII email; surrounding ASCII whitespace is stripped and the domain is lowercased"
            }),
        );
        set_property(
            schemas,
            schema,
            "phoneNumber",
            json!({
                "type": "string",
                "pattern": PHONE_NUMBER_INPUT_PATTERN,
                "description": "E.164 phone number after stripping surrounding ASCII whitespace"
            }),
        );
    }
    set_property(
        schemas,
        "Profile",
        "contactEmail",
        json!({
            "type": "string",
            "minLength": 3,
            "maxLength": 254,
            "pattern": CONTACT_EMAIL_PATTERN
        }),
    );
    set_property(schemas, "Profile", "phoneNumber", canonical_phone_schema());
    if let Some(profile) = schemas.get_mut("Profile").and_then(Value::as_object_mut) {
        profile.insert(
            "description".to_owned(),
            json!("updatedAt is greater than or equal to createdAt"),
        );
    }
    if let Some(update) = schemas
        .get_mut("ProfileUpdate")
        .and_then(Value::as_object_mut)
    {
        update.insert("minProperties".to_owned(), json!(1));
    }
    set_property(
        schemas,
        "ProfileUpdate",
        "marketingOptIn",
        json!({"type":"boolean"}),
    );
}

fn strengthen_item_schemas(schemas: &mut Value) {
    let item_ids = (1..=30)
        .map(|index| format!("item-{index:03}"))
        .collect::<Vec<_>>();
    set_property(
        schemas,
        "Item",
        "id",
        json!({"type":"string", "enum":item_ids}),
    );
    set_property(schemas, "Item", "name", bounded_name_schema());
    set_property(
        schemas,
        "Item",
        "description",
        json!({"type":"string", "minLength":1, "maxLength":500}),
    );
    set_array_max_items(schemas, "ItemPage", "items", 100);
}

fn strengthen_github_schemas(schemas: &mut Value) {
    set_required(
        schemas,
        "Owner",
        &[
            "id",
            "login",
            "type",
            "name",
            "avatarUrl",
            "htmlUrl",
            "company",
            "blog",
            "location",
            "bio",
            "publicRepos",
            "followers",
            "following",
            "createdAt",
            "updatedAt",
        ],
    );
    for property in ["login", "type"] {
        set_property(schemas, "Owner", property, non_empty_string_schema());
    }
    for property in ["name", "company", "blog", "location", "bio"] {
        set_property(
            schemas,
            "Owner",
            property,
            nullable_non_empty_string_schema(),
        );
    }
    for property in ["avatarUrl", "htmlUrl"] {
        set_property(schemas, "Owner", property, absolute_http_uri_schema());
    }
    for property in ["createdAt", "updatedAt"] {
        set_property(schemas, "Owner", property, timestamp_schema());
    }

    set_required(
        schemas,
        "RepositorySummary",
        &["id", "name", "fullName", "description", "htmlUrl", "fork"],
    );
    strengthen_repository_properties(schemas, "RepositorySummary");

    set_required(
        schemas,
        "Repository",
        &[
            "id",
            "name",
            "fullName",
            "description",
            "htmlUrl",
            "fork",
            "language",
            "stargazersCount",
            "forksCount",
            "openIssuesCount",
            "archived",
            "createdAt",
            "updatedAt",
            "pushedAt",
            "defaultBranch",
            "license",
            "topics",
            "disabled",
        ],
    );
    strengthen_repository_properties(schemas, "Repository");
    for property in ["language", "license"] {
        set_property(
            schemas,
            "Repository",
            property,
            nullable_non_empty_string_schema(),
        );
    }
    for property in ["createdAt", "updatedAt"] {
        set_property(schemas, "Repository", property, timestamp_schema());
    }
    set_property(
        schemas,
        "Repository",
        "pushedAt",
        json!({
            "oneOf": [timestamp_schema(), {"type":"null"}]
        }),
    );
    set_property(
        schemas,
        "Repository",
        "defaultBranch",
        non_empty_string_schema(),
    );
    if let Some(topics) = schema_property_mut(schemas, "Repository", "topics") {
        topics["uniqueItems"] = json!(true);
    }

    set_required(
        schemas,
        "Activity",
        &[
            "id",
            "actor",
            "actorAvatarUrl",
            "ref",
            "timestamp",
            "activityType",
        ],
    );
    set_property(
        schemas,
        "Activity",
        "actor",
        nullable_non_empty_string_schema(),
    );
    set_property(
        schemas,
        "Activity",
        "actorAvatarUrl",
        json!({
            "oneOf": [absolute_http_uri_schema(), {"type":"null"}]
        }),
    );
    for property in ["ref", "activityType"] {
        set_property(schemas, "Activity", property, non_empty_string_schema());
    }
    set_property(schemas, "Activity", "timestamp", timestamp_schema());
    if let Some(activity) = schemas.get_mut("Activity").and_then(Value::as_object_mut) {
        activity.insert(
            "allOf".to_owned(),
            json!([{
                "if": {"properties":{"actor":{"type":"null"}}, "required":["actor"]},
                "then": {"properties":{"actorAvatarUrl":{"type":"null"}}},
                "else": {"properties":{"actorAvatarUrl":absolute_http_uri_schema()}}
            }]),
        );
    }

    set_property(schemas, "Language", "name", non_empty_string_schema());
    set_property(schemas, "Tag", "name", non_empty_string_schema());
    set_property(
        schemas,
        "TagCommit",
        "sha",
        json!({"type":"string", "pattern":r"^(?:[0-9a-f]{40}|[0-9a-f]{64})$"}),
    );
    for (schema, property) in [
        ("GitHubRepositoryPage", "repos"),
        ("GitHubActivityPage", "activities"),
        ("GitHubTagPage", "tags"),
    ] {
        set_array_max_items(schemas, schema, property, 100);
    }
}

fn strengthen_repository_properties(schemas: &mut Value, schema: &str) {
    for property in ["name", "fullName"] {
        set_property(schemas, schema, property, non_empty_string_schema());
    }
    set_property(
        schemas,
        schema,
        "description",
        nullable_non_empty_string_schema(),
    );
    set_property(schemas, schema, "htmlUrl", absolute_http_uri_schema());
}

fn strengthen_problem_schemas(schemas: &mut Value) {
    set_property(
        schemas,
        "ProblemDetails",
        "type",
        json!({"type":"string", "const":"about:blank"}),
    );
    set_property(
        schemas,
        "ProblemDetails",
        "errors",
        json!({
            "type":"array",
            "minItems":1,
            "maxItems":32,
            "items":{"$ref":"#/components/schemas/ProblemIssue"}
        }),
    );
    set_property(
        schemas,
        "ProblemIssue",
        "detail",
        json!({"type":"string", "minLength":1, "maxLength":200}),
    );
    set_property(
        schemas,
        "ProblemIssue",
        "source",
        json!({"$ref":"#/components/schemas/ProblemSource"}),
    );
    schemas["ProblemSource"] = json!({
        "oneOf": [
            closed_single_string_property("pointer"),
            closed_single_string_property("parameter"),
            closed_single_string_property("header")
        ]
    });
}

fn closed_single_string_property(name: &str) -> Value {
    let mut schema = json!({
        "type":"object",
        "additionalProperties":false,
        "properties":{},
        "required":[name]
    });
    schema["properties"][name] = json!({"type":"string", "minLength":1, "maxLength":256});
    schema
}

fn bounded_name_schema() -> Value {
    json!({
        "type":"string",
        "minLength":1,
        "maxLength":100,
        "pattern":BOUNDED_NAME_PATTERN
    })
}

fn opaque_id_schema() -> Value {
    json!({"type":"string", "minLength":1, "maxLength":128})
}

fn timestamp_schema() -> Value {
    json!({"type":"string", "format":"date-time", "pattern":TIMESTAMP_PATTERN})
}

fn canonical_phone_schema() -> Value {
    json!({
        "type":"string",
        "minLength":8,
        "maxLength":16,
        "pattern":PHONE_NUMBER_PATTERN
    })
}

fn non_empty_string_schema() -> Value {
    json!({"type":"string", "minLength":1})
}

fn nullable_non_empty_string_schema() -> Value {
    json!({"type":["string", "null"], "minLength":1})
}

fn absolute_http_uri_schema() -> Value {
    json!({
        "type":"string",
        "format":"uri",
        "pattern":r"^[Hh][Tt][Tt][Pp][Ss]?://"
    })
}

fn set_required(schemas: &mut Value, schema: &str, required: &[&str]) {
    if let Some(object) = schemas.get_mut(schema).and_then(Value::as_object_mut) {
        object.insert("required".to_owned(), json!(required));
    }
}

fn schema_property_mut<'a>(
    schemas: &'a mut Value,
    schema: &str,
    property: &str,
) -> Option<&'a mut Value> {
    schemas
        .get_mut(schema)?
        .get_mut("properties")?
        .get_mut(property)
}

fn set_array_max_items(schemas: &mut Value, schema: &str, property: &str, maximum: u64) {
    if let Some(array) = schema_property_mut(schemas, schema, property) {
        array["maxItems"] = json!(maximum);
    }
}

fn set_property(schemas: &mut Value, schema: &str, property: &str, value: Value) {
    if let Some(properties) = schemas
        .get_mut(schema)
        .and_then(|value| value.get_mut("properties"))
        .and_then(Value::as_object_mut)
    {
        properties.insert(property.to_owned(), value);
    }
}

fn strengthen_operations(document: &mut Value) {
    let Some(paths) = document["paths"].as_object_mut() else {
        return;
    };
    for (path_name, path) in paths {
        let Some(methods) = path.as_object_mut() else {
            continue;
        };
        for (method, operation) in methods {
            let Some(operation) = operation.as_object_mut() else {
                continue;
            };
            if path_name != "/v1/profile" {
                operation.insert("security".to_owned(), json!([]));
            }
            let parameters = operation
                .entry("parameters")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .expect("operation parameters should be an array");
            parameters.push(request_id_parameter());
            for parameter in parameters.iter_mut() {
                let Some(name) = parameter.get("name").and_then(Value::as_str) else {
                    continue;
                };
                if name == "limit" {
                    parameter["schema"]["default"] = json!(20);
                } else if name == "cursor" {
                    parameter["schema"]["minLength"] = json!(1);
                    parameter["schema"]["maxLength"] = json!(2048);
                    parameter["schema"]["pattern"] = json!(r"^[!-~]+$");
                } else if name == "owner" {
                    parameter["schema"]["minLength"] = json!(1);
                    parameter["schema"]["maxLength"] = json!(39);
                    parameter["schema"]["pattern"] =
                        json!(r"^[A-Za-z0-9](?:[A-Za-z0-9_-]{0,37}[A-Za-z0-9])?$");
                } else if name == "repo" {
                    parameter["schema"]["minLength"] = json!(1);
                    parameter["schema"]["maxLength"] = json!(100);
                    parameter["schema"]["pattern"] = json!(r"^(?=.*[A-Za-z0-9_-])[A-Za-z0-9._-]+$");
                } else if name == "category" {
                    parameter["description"] = json!("Exact category filter; the query is closed");
                    parameter["schema"]["enum"] = json!([
                        "electronics",
                        "tools",
                        "accessories",
                        "robotics",
                        "power",
                        "components"
                    ]);
                }
            }

            if matches!(
                (method.as_str(), path_name.as_str()),
                ("post", "/v1/hello" | "/v1/profile") | ("patch", "/v1/profile")
            ) {
                operation["requestBody"]["required"] = json!(true);
            }
            let Some(responses) = operation
                .get_mut("responses")
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            for (status, response) in responses {
                add_common_response_headers(response, !(method == "delete" && status == "204"));
                if let Some(code) = problem_code_for_response(path_name, status) {
                    for media_type in ["application/problem+json", "application/cbor"] {
                        if response["content"].get(media_type).is_some() {
                            response["content"][media_type]["schema"] = exact_problem_schema(code);
                        }
                    }
                }
                if status == "429" {
                    response["headers"]["Retry-After"] = integer_header("Retry delay in seconds");
                    response["headers"]["X-RateLimit-Reset"] =
                        integer_header("Optional validated GitHub reset epoch");
                }
                if status == "401" {
                    response["headers"]["WWW-Authenticate"] = literal_header("Bearer");
                }
                if method == "post" && path_name == "/v1/profile" && status == "201" {
                    response["headers"]["Location"] = literal_header("/v1/profile");
                }
            }
        }
    }
}

fn problem_code_for_response(path: &str, status: &str) -> Option<ProblemCode> {
    Some(match status {
        "400" => ProblemCode::InvalidRequest,
        "401" => ProblemCode::Unauthorized,
        "404" if path == "/v1/profile" => ProblemCode::ProfileNotFound,
        "404" => ProblemCode::GithubNotFound,
        "406" => ProblemCode::NotAcceptable,
        "409" => ProblemCode::ProfileExists,
        "413" => ProblemCode::PayloadTooLarge,
        "415" => ProblemCode::UnsupportedMediaType,
        "422" => ProblemCode::ValidationFailed,
        "429" => ProblemCode::GithubRateLimit,
        "500" => ProblemCode::InternalError,
        "502" => ProblemCode::GithubUpstream,
        "503" => ProblemCode::DependencyUnavailable,
        "504" => ProblemCode::GithubTimeout,
        _ => return None,
    })
}

fn exact_problem_schema(code: ProblemCode) -> Value {
    json!({
        "type":"object",
        "additionalProperties":false,
        "properties":{
            "type":{"type":"string", "const":"about:blank"},
            "title":{"type":"string", "const":code.title()},
            "status":{"type":"integer", "const":code.status().as_u16()},
            "detail":{"type":"string", "const":code.detail()},
            "code":{"type":"string", "const":problem_code_name(code)},
            "errors":{
                "type":"array",
                "minItems":1,
                "maxItems":32,
                "items":{"$ref":"#/components/schemas/ProblemIssue"}
            }
        },
        "required":["title", "status", "detail", "code"]
    })
}

const fn problem_code_name(code: ProblemCode) -> &'static str {
    match code {
        ProblemCode::InvalidRequest => "invalid_request",
        ProblemCode::Unauthorized => "unauthorized",
        ProblemCode::Forbidden => "forbidden",
        ProblemCode::ClientGeneratedIdUnsupported => "client_generated_id_unsupported",
        ProblemCode::RelationshipsUnsupported => "relationships_unsupported",
        ProblemCode::NotFound => "not_found",
        ProblemCode::ProfileNotFound => "profile_not_found",
        ProblemCode::GithubNotFound => "github_not_found",
        ProblemCode::MethodNotAllowed => "method_not_allowed",
        ProblemCode::NotAcceptable => "not_acceptable",
        ProblemCode::ProfileExists => "profile_exists",
        ProblemCode::ProfileResourceMismatch => "profile_resource_mismatch",
        ProblemCode::PayloadTooLarge => "payload_too_large",
        ProblemCode::UnsupportedMediaType => "unsupported_media_type",
        ProblemCode::ValidationFailed => "validation_failed",
        ProblemCode::RateLimited => "rate_limited",
        ProblemCode::GithubRateLimit => "github_rate_limit",
        ProblemCode::InternalError => "internal_error",
        ProblemCode::GithubUpstream => "github_upstream",
        ProblemCode::DependencyUnavailable => "dependency_unavailable",
        ProblemCode::GithubTimeout => "github_timeout",
    }
}

fn request_id_parameter() -> Value {
    json!({
        "name": "X-Request-ID",
        "in": "header",
        "required": false,
        "description": "Missing, invalid, repeated, or comma-combined values are replaced",
        "schema": {
            "type": "string",
            "minLength": 1,
            "maxLength": 128,
            "pattern": "^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$"
        }
    })
}

fn add_common_response_headers(response: &mut Value, vary: bool) {
    response["headers"]["X-Request-ID"] = request_id_header();
    response["headers"]["Cache-Control"] = literal_header("no-store");
    response["headers"]["X-Content-Type-Options"] = literal_header("nosniff");
    response["headers"]["X-Frame-Options"] = literal_header("DENY");
    response["headers"]["Referrer-Policy"] = literal_header("strict-origin-when-cross-origin");
    if vary {
        response["headers"]["Vary"] = string_header("Includes Accept");
    }
}

fn request_id_header() -> Value {
    json!({
        "description":"Selected request correlation identifier",
        "schema":{
            "type":"string",
            "minLength":1,
            "maxLength":128,
            "pattern":"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$"
        }
    })
}

fn string_header(description: &str) -> Value {
    json!({"description":description, "schema":{"type":"string"}})
}

fn literal_header(value: &str) -> Value {
    json!({"schema":{"type":"string", "const":value}})
}

fn integer_header(description: &str) -> Value {
    json!({
        "description": description,
        "schema": {"type":"integer", "minimum":0, "maximum":SAFE_INTEGER_MAX}
    })
}

#[cfg(test)]
mod tests {
    use super::openapi_document;

    #[test]
    fn generated_document_has_exact_inventory_and_security() {
        let document = openapi_document();
        let expected = [
            ("/health", "get", "getHealth"),
            ("/v1/hello", "get", "getHello"),
            ("/v1/hello", "post", "createHello"),
            ("/v1/items", "get", "listItems"),
            ("/v1/profile", "post", "createProfile"),
            ("/v1/profile", "get", "getProfile"),
            ("/v1/profile", "patch", "updateProfile"),
            ("/v1/profile", "delete", "deleteProfile"),
            ("/v1/github/owners/{owner}", "get", "getGitHubOwner"),
            (
                "/v1/github/owners/{owner}/repos",
                "get",
                "listGitHubOwnerRepositories",
            ),
            (
                "/v1/github/repos/{owner}/{repo}",
                "get",
                "getGitHubRepository",
            ),
            (
                "/v1/github/repos/{owner}/{repo}/activity",
                "get",
                "listGitHubRepositoryActivity",
            ),
            (
                "/v1/github/repos/{owner}/{repo}/languages",
                "get",
                "listGitHubRepositoryLanguages",
            ),
            (
                "/v1/github/repos/{owner}/{repo}/tags",
                "get",
                "listGitHubRepositoryTags",
            ),
        ];
        assert_eq!(document["openapi"], "3.1.0");
        for (path, method, operation_id) in expected {
            assert_eq!(document["paths"][path][method]["operationId"], operation_id);
        }
        assert_eq!(
            document["paths"]["/health"]["get"]["security"],
            serde_json::json!([])
        );
        assert_eq!(
            document["paths"]["/v1/profile"]["get"]["security"],
            serde_json::json!([{"bearerAuth":[]}])
        );
    }
}

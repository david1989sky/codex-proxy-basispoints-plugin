use super::response::JSON_CONTENT_TYPE;
use gateway_plugin_sdk::call::management::{ManagementRegistration, ManagementRoute};

pub(crate) fn registration() -> ManagementRegistration {
    let get = |path: &str| ManagementRoute {
        method: "GET".to_owned(),
        path: path.to_owned(),
        request_content_types: Vec::new(),
        response_content_types: vec![JSON_CONTENT_TYPE.to_owned()],
    };
    ManagementRegistration {
        routes: vec![
            get("api/status"),
            ManagementRoute {
                method: "POST".to_owned(),
                path: "api/basispoints/responses".to_owned(),
                request_content_types: vec![JSON_CONTENT_TYPE.to_owned()],
                response_content_types: vec![
                    JSON_CONTENT_TYPE.to_owned(),
                    "text/event-stream".to_owned(),
                ],
            },
        ],
        resources: Vec::new(),
        pages: Vec::new(),
        callbacks: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::registration;

    #[test]
    fn registers_basispoints_responses_route_contract() {
        let registration = registration();
        let route = registration
            .routes
            .iter()
            .find(|route| route.method == "POST" && route.path == "api/basispoints/responses")
            .expect("Basis Points route");

        assert_eq!(route.request_content_types, vec!["application/json"]);
        assert_eq!(
            route.response_content_types,
            vec!["application/json", "text/event-stream"]
        );
        assert_eq!(registration.routes.len(), 2);
        assert!(registration.resources.is_empty());
        assert!(registration.pages.is_empty());
    }
}

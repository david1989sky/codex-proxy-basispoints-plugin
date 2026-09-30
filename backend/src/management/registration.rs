use super::response::JSON_CONTENT_TYPE;
use gateway_plugin_sdk::call::management::{
    ManagementPage, ManagementRegistration, ManagementResource, ManagementRoute,
};

pub(crate) fn registration() -> ManagementRegistration {
    let get = |path: &str| ManagementRoute {
        method: "GET".to_owned(),
        path: path.to_owned(),
        request_content_types: Vec::new(),
        response_content_types: vec![JSON_CONTENT_TYPE.to_owned()],
    };
    let post = |path: &str| ManagementRoute {
        method: "POST".to_owned(),
        path: path.to_owned(),
        request_content_types: vec![JSON_CONTENT_TYPE.to_owned()],
        response_content_types: vec![JSON_CONTENT_TYPE.to_owned()],
    };
    ManagementRegistration {
        routes: vec![
            get("api/status"),
            get("api/migration"),
            post("api/migration/import"),
            post("api/request"),
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
        resources: ["web/index.html", "web/app.js", "web/app.css"]
            .into_iter()
            .map(|path| ManagementResource {
                path: path.to_owned(),
                public: false,
            })
            .collect(),
        pages: vec![ManagementPage {
            id: "twofa".to_owned(),
            title: "批量 2FA 授权".to_owned(),
            description: Some("批量导入、自动授权与重新授权".to_owned()),
            entry: "web/index.html".to_owned(),
            icon: None,
        }],
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
    }
}

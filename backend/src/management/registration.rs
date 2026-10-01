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
    ManagementRegistration {
        routes: vec![
            get("api/status"),
            get("api/usage"),
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
        resources: vec![
            ManagementResource {
                path: "web/index.html".to_owned(),
                public: false,
            },
            ManagementResource {
                path: "web/app.js".to_owned(),
                public: false,
            },
            ManagementResource {
                path: "web/app.css".to_owned(),
                public: false,
            },
        ],
        pages: vec![ManagementPage {
            id: "usage".to_owned(),
            title: "BPS 通道使用情况".to_owned(),
            description: Some("查看当前插件进程的 BPS 请求统计".to_owned()),
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
        assert!(
            registration
                .routes
                .iter()
                .any(|route| route.method == "GET" && route.path == "api/status")
        );
        assert!(
            registration
                .routes
                .iter()
                .any(|route| route.method == "GET" && route.path == "api/usage")
        );
        assert_eq!(registration.routes.len(), 3);
        assert_eq!(
            registration
                .resources
                .iter()
                .map(|resource| resource.path.as_str())
                .collect::<Vec<_>>(),
            vec!["web/index.html", "web/app.js", "web/app.css"]
        );
        assert!(
            registration
                .resources
                .iter()
                .all(|resource| !resource.public)
        );
        assert_eq!(registration.pages.len(), 1);
        let page = &registration.pages[0];
        assert_eq!(page.id, "usage");
        assert_eq!(page.entry, "web/index.html");
        assert_eq!(page.title, "BPS 通道使用情况");
    }
}

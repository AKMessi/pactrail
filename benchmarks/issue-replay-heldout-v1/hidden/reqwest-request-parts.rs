
#[cfg(test)]
mod hidden_issue_2989_tests {
    use super::*;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct RequestMarker(&'static str);

    #[test]
    fn try_from_http_request_preserves_version_and_extensions() {
        let mut source = HttpRequest::builder()
            .method(Method::POST)
            .uri("https://example.com/resource")
            .version(Version::HTTP_2)
            .body(Vec::from("body"))
            .expect("valid HTTP request");
        source
            .extensions_mut()
            .insert(RequestMarker("keep-me"));

        let converted = Request::try_from(source).expect("valid reqwest request");

        assert_eq!(converted.version(), Version::HTTP_2);
        assert_eq!(
            converted.inner.extensions().get::<RequestMarker>(),
            Some(&RequestMarker("keep-me"))
        );
    }
}

#[test]
fn cloud_build_uses_guaranteed_identity_and_pushes_before_deploying() {
    let config = include_str!("../cloudbuild.yaml");

    assert!(!config.contains("SHORT_SHA"));
    assert!(config.contains(":${BUILD_ID}"));
    assert!(config.contains("--build-arg VERSION=\"${BUILD_ID}\""));
    assert!(config.contains("APP_ENVIRONMENT=production,FIREBASE_PROJECT_ID=${PROJECT_ID}"));

    let push = config
        .find("docker push \"${IMAGE_URI}\"")
        .expect("the immutable image must be pushed");
    let deploy = config
        .find("gcloud run deploy")
        .expect("the deploy command must exist");
    assert!(push < deploy, "the image must be pushed before deployment");
}

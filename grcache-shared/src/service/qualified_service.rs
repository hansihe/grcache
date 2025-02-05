use std::fmt::Display;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to parse qualified service name: {name}")]
    FailedParseQualifiedService { name: String },
}

#[derive(Debug, Clone)]
pub struct QualifiedService {
    pub package: String,
    pub name: String,
}

impl Display for QualifiedService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.package, self.name)
    }
}

impl QualifiedService {
    pub fn parse(qualified_service: &str) -> Result<QualifiedService, Error> {
        match qualified_service.rsplit_once(".") {
            Some((package, service_name)) => Ok(QualifiedService {
                package: package.into(),
                name: service_name.into(),
            }),
            None => Err(Error::FailedParseQualifiedService {
                name: qualified_service.into(),
            }),
        }
    }
}

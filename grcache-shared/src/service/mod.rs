use std::collections::HashMap;

use protobuf::{
    descriptor::FileDescriptorSet,
    reflect::{FileDescriptor, MethodDescriptor, ServiceDescriptor},
};
use qualified_service::QualifiedService;

use crate::{field_ref::FieldRef, protos::options::GrcacheMethodOptions};

pub mod descriptor_set;
pub mod qualified_service;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to load descriptor set")]
    DescriptorError(#[from] protobuf::Error),
    #[error("service not found in descriptor set: {service}")]
    ServiceNotFound { service: QualifiedService },
}

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("`cache_ttl` set to invalid value {value}, must be zero or positive")]
    InvalidCacheTTL { value: i32 },
    #[error("`hash_on` `{field_ref}` was invalid field ref")]
    HashOnInvalidFieldRef {
        field_ref: String,
        #[source]
        source: crate::field_ref::ParseError,
    },
    #[error("`hash_on` field not found in request: {field}")]
    HashOnFieldNotFound { field: String },
}

#[derive(Debug)]
pub struct CacheSpec {
    pub hash_on: Vec<FieldRef>,
    pub descriptor: GrcacheMethodOptions,
}

pub struct MethodSpec {
    pub name: String,
    pub descriptor: MethodDescriptor,
    /// Only present if caching options are specified.
    pub cache_spec: Option<CacheSpec>,
}

impl MethodSpec {
    fn build(method: MethodDescriptor, mut validation_error: impl FnMut(ValidationError)) -> Self {
        let cache_spec = crate::protos::options::exts::grcache
            .get(&method.proto().options)
            .and_then(|opt| {
                let mut success = true;
                let hash_on: Vec<FieldRef> = Vec::new();

                if opt.cache_ttl < 0 {
                    validation_error(ValidationError::InvalidCacheTTL {
                        value: opt.cache_ttl,
                    });
                    success = false;
                }

                //for field_ref_str in opt.hash_on.iter() {
                //    let field_ref = crate::field_ref::FieldRef::parse(field_ref_str);
                //    match field_ref {
                //        Ok(field_ref) => {
                //            // TODO validate field ref
                //            field_ref.validate(&method.input_type());

                //            hash_on.push(field_ref);
                //        }
                //        Err(parse_err) => {
                //            validation_error(ValidationError::HashOnInvalidFieldRef {
                //                field_ref: field_ref_str.into(),
                //                source: parse_err,
                //            });
                //            success = false;
                //        }
                //    }
                //}

                if success {
                    Some(CacheSpec {
                        hash_on,
                        descriptor: opt,
                    })
                } else {
                    None
                }
            });

        MethodSpec {
            name: method.proto().name().into(),
            descriptor: method,
            cache_spec,
        }
    }
}

pub struct ServiceSpec {
    pub name: QualifiedService,
    pub passthrough: bool,
    pub methods: HashMap<String, MethodSpec>,
}

impl ServiceSpec {
    pub fn build_passthrough(service: &QualifiedService) -> Self {
        ServiceSpec {
            name: service.clone(),
            passthrough: true,
            methods: HashMap::new(),
        }
    }

    pub fn build(
        descriptor_set: &FileDescriptorSet,
        service: &QualifiedService,
    ) -> Result<(Self, Vec<ValidationError>), Error> {
        let mut dyns = Vec::new();
        for descr in descriptor_set.file.iter().cloned() {
            dyns.push(FileDescriptor::new_dynamic(descr, &dyns)?);
        }

        let service_descriptor = find_service(&dyns, service)?;

        let mut validation_errors = Vec::new();
        let mut validation_error = |err| {
            validation_errors.push(err);
        };

        let methods: HashMap<_, _> = service_descriptor
            .methods()
            .map(|method| {
                let spec = MethodSpec::build(method, &mut validation_error);
                (spec.name.clone(), spec)
            })
            .collect();

        let service = ServiceSpec {
            name: service.clone(),
            passthrough: false,
            methods,
        };

        Ok((service, validation_errors))
    }
}

fn find_service(
    descriptor_set: &[FileDescriptor],
    service: &QualifiedService,
) -> Result<ServiceDescriptor, Error> {
    let service_descriptor = descriptor_set
        .iter()
        .flat_map(|f| f.services().map(|s| (f.package(), s)))
        .find(|(p, s)| *p == service.package && s.proto().name() == service.name)
        .map(|(_p, s)| s)
        .ok_or_else(|| Error::ServiceNotFound {
            service: service.clone(),
        })?;

    Ok(service_descriptor)
}

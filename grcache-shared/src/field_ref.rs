use protobuf::reflect::MessageDescriptor;

#[derive(Debug)]
pub enum PathComponent {
    Field(String),
}

#[derive(Debug)]
pub struct FieldRef {
    pub components: Vec<PathComponent>,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {}

impl FieldRef {
    pub fn parse(string: &str) -> Result<Self, ParseError> {
        let components: Vec<_> = string
            .split(".")
            .map(|elem| PathComponent::Field(elem.into()))
            .collect();

        Ok(FieldRef { components })
    }

    pub fn validate(&self, descriptor: &MessageDescriptor) {
        // TODO
    }
}

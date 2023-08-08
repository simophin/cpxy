use super::PropertyValue;
use ipnetwork::IpNetwork;

impl<'a> From<&'a str> for PropertyValue<'a> {
    fn from(value: &'a str) -> Self {
        Self::String(value)
    }
}

impl<'a> From<IpNetwork> for PropertyValue<'a> {
    fn from(value: IpNetwork) -> Self {
        Self::IPNetwork(value)
    }
}

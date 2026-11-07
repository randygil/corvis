//! Serde helpers used by generated models.

/// `Option<Option<T>>` for nullable to-one relations: absent key = `None` (not
/// included), `null` = `Some(None)` (included, no row), value = `Some(Some(v))`.
///
/// ```ignore
/// #[serde(default, skip_serializing_if = "Option::is_none", with = "::corvis_orm::serde_helpers::double_option")]
/// pub profile: Option<Option<Box<Profile>>>,
/// ```
pub mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<T: Serialize, S: Serializer>(v: &Option<Option<T>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            None | Some(None) => s.serialize_none(),
            Some(Some(x)) => x.serialize(s),
        }
    }

    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(d).map(Some)
    }
}

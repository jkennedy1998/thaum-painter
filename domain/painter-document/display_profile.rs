//! Editing-facing projection of the saved renderer display choices.
//!
//! `file-schema` remains the canonical persistence owner. This module gives
//! editor/session callers a small owned record so they do not need to interpret
//! saved-file layout or invent defaults themselves.

use crate::file_schema::FileSchema;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentDisplayProfile {
    pub cell_width: u32,
    pub cell_height: u32,
    pub typeface_id: String,
    pub spriteset_id: String,
}

/// Projects the document's saved four-axis display profile for editing UI.
pub fn document_display_profile(schema: &FileSchema) -> DocumentDisplayProfile {
    let profile = &schema.document.display_profile;
    DocumentDisplayProfile {
        cell_width: profile.cell_width,
        cell_height: profile.cell_height,
        typeface_id: profile.typeface_id.clone(),
        spriteset_id: profile.spriteset_id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_schema::parse_file_schema_from_str;

    const EXAMPLE_FILE_SCHEMA_JSON: &str =
        include_str!("../file/file-schema/example-thaum-painter-file-v3.json");

    #[test]
    fn projects_the_saved_display_profile_without_becoming_the_storage_owner() {
        let schema = parse_file_schema_from_str(EXAMPLE_FILE_SCHEMA_JSON).unwrap();
        assert_eq!(
            document_display_profile(&schema),
            DocumentDisplayProfile {
                cell_width: 12,
                cell_height: 16,
                typeface_id: "thaum-mono".to_owned(),
                spriteset_id: "monothaum-atlas-v3".to_owned(),
            }
        );
    }
}

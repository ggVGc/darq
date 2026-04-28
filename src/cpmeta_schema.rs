//! Schema definitions for ICOS Carbon Portal metadata tables (34 tables).
//!
//! Generated from `cpmeta_tables.sql`.
//!
//! The SQL tables use `rdf_subject` as the subject IRI column. Configure
//! the SQL engine accordingly:
//!
//! ```rust,ignore
//! let engine = SqlEngine::new(&executor).with_subject_column("rdf_subject");
//! ```

use crate::rdf::Iri;
use crate::schema::{FieldType, Schema};

// ---------------------------------------------------------------------------
// Domain-specific namespace helpers
// ---------------------------------------------------------------------------

/// Type IRI namespace. Local part must match the SQL table name so that
/// `table_name()` in the SQL engine produces the right identifier.
macro_rules! tbl {
    ($t:literal) => {
        concat!("http://meta.icos-cp.eu/tables/", $t)
    };
}

macro_rules! cpmeta {
    ($p:literal) => {
        concat!("http://meta.icos-cp.eu/ontologies/cpmeta/", $p)
    };
}

// ---------------------------------------------------------------------------
// Tables (in dependency order matching cpmeta_tables.sql)
// ---------------------------------------------------------------------------

define_resource!(
    /// cpmeta:QuantityKind (21 instances)
    QuantityKind, tbl!("ct_quantity_kinds"), [
        ("label",   rdfs!("label"),   FieldType::String),
        ("comment", rdfs!("comment"), FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:CentralFacility (2 instances)
    CentralFacility, tbl!("ct_central_facilities"), [
        ("has_name", cpmeta!("hasName"), FieldType::String),
        ("label",    rdfs!("label"),     FieldType::String),
        ("comment",  rdfs!("comment"),   FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:SpecificDatasetType (2 instances)
    SpecificDatasetType, tbl!("ct_specific_dataset_types"), [
        ("label", rdfs!("label"), FieldType::String),
    ]
);

define_resource!(
    /// UNION: cpmeta:Organization (256 instances)
    /// Discriminator column `org_type` IN ('organization', 'thematic_center', 'central_facility')
    Organization, tbl!("ct_organizations"), [
        ("has_name",   cpmeta!("hasName"),  FieldType::String),
        ("label",      rdfs!("label"),      FieldType::String),
        ("has_atc_id", cpmeta!("hasAtcId"), FieldType::String),
        ("has_otc_id", cpmeta!("hasOtcId"), FieldType::String),
        ("has_etc_id", cpmeta!("hasEtcId"), FieldType::String),
        ("see_also",   rdfs!("seeAlso"),    FieldType::String),
        ("has_email",  cpmeta!("hasEmail"), FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:Funder (47 instances)
    Funder, tbl!("ct_funders"), [
        ("has_etc_id", cpmeta!("hasEtcId"), FieldType::String),
        ("has_name",   cpmeta!("hasName"),  FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:EcosystemType (17 instances)
    EcosystemType, tbl!("ct_ecosystem_types"), [
        ("label",   rdfs!("label"),   FieldType::String),
        ("comment", rdfs!("comment"), FieldType::String),
    ]
);

define_resource!(
    /// UNION: cpmeta:SpatialCoverage, LatLonBox, Position (4,164 instances)
    /// Discriminator column `coverage_type` IN ('spatial', 'latlon', 'position')
    SpatialCoverage, tbl!("ct_spatial_coverages"), [
        ("as_geo_json",       cpmeta!("asGeoJSON"),        FieldType::String),
        ("label",             rdfs!("label"),               FieldType::String),
        ("has_eastern_bound", cpmeta!("hasEasternBound"),   FieldType::Double),
        ("has_northern_bound", cpmeta!("hasNorthernBound"), FieldType::Double),
        ("has_southern_bound", cpmeta!("hasSouthernBound"), FieldType::Double),
        ("has_western_bound", cpmeta!("hasWesternBound"),   FieldType::Double),
        ("has_latitude",      cpmeta!("hasLatitude"),       FieldType::Double),
        ("has_longitude",     cpmeta!("hasLongitude"),      FieldType::Double),
    ]
);

define_resource!(
    /// cpmeta:Project (12 instances)
    Project, tbl!("ct_projects"), [
        ("comment",                      rdfs!("comment"),                    FieldType::String),
        ("label",                        rdfs!("label"),                      FieldType::String),
        ("see_also",                     rdfs!("seeAlso"),                    FieldType::String),
        ("has_keywords",                 cpmeta!("hasKeywords"),              FieldType::String),
        ("has_hide_from_search_policy",  cpmeta!("hasHideFromSearchPolicy"), FieldType::Boolean),
        ("has_skip_pid_minting_policy",  cpmeta!("hasSkipPidMintingPolicy"), FieldType::Boolean),
        ("has_skip_storage_policy",      cpmeta!("hasSkipStoragePolicy"),    FieldType::Boolean),
    ]
);

define_resource!(
    /// cpmeta:VariableInfo (4,957 instances)
    VariableInfo, tbl!("ct_variable_infos"), [
        ("label",         rdfs!("label"),          FieldType::String),
        ("has_max_value", cpmeta!("hasMaxValue"),  FieldType::Double),
        ("has_min_value", cpmeta!("hasMinValue"),  FieldType::Double),
    ]
);

define_resource!(
    /// cpmeta:LinkBox (158 instances)
    LinkBox, tbl!("ct_link_boxes"), [
        ("has_cover_image",  cpmeta!("hasCoverImage"),  FieldType::String),
        ("has_name",         cpmeta!("hasName"),         FieldType::String),
        ("has_order_weight", cpmeta!("hasOrderWeight"),  FieldType::Integer),
        ("label",            rdfs!("label"),             FieldType::String),
        ("has_webpage_link", cpmeta!("hasWebpageLink"),  FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:ClimateZone (30 instances)
    ClimateZone, tbl!("ct_climate_zones"), [
        ("label",    rdfs!("label"),   FieldType::String),
        ("see_also", rdfs!("seeAlso"), FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:Role (5 instances)
    Role, tbl!("ct_roles"), [
        ("label",   rdfs!("label"),   FieldType::String),
        ("comment", rdfs!("comment"), FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:DataTheme (4 instances)
    DataTheme, tbl!("ct_data_themes"), [
        ("has_icon",        cpmeta!("hasIcon"),       FieldType::String),
        ("has_marker_icon", cpmeta!("hasMarkerIcon"), FieldType::String),
        ("label",           rdfs!("label"),           FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:ObjectEncoding (3 instances)
    ObjectEncoding, tbl!("ct_object_encodings"), [
        ("label", rdfs!("label"), FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:ValueFormat (13 instances)
    ValueFormat, tbl!("ct_value_formats"), [
        ("label",   rdfs!("label"),   FieldType::String),
        ("comment", rdfs!("comment"), FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:ValueType (166 instances)
    ValueType, tbl!("ct_value_types"), [
        ("label",            rdfs!("label"),              FieldType::String),
        ("has_quantity_kind", cpmeta!("hasQuantityKind"), ref_to!(tbl!("ct_quantity_kinds"))),
        ("has_unit",         cpmeta!("hasUnit"),          FieldType::String),
        ("comment",          rdfs!("comment"),            FieldType::String),
        ("exact_match",      skos!("exactMatch"),         FieldType::String),
        ("see_also",         rdfs!("seeAlso"),            FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:Instrument (4,826 instances)
    Instrument, tbl!("ct_instruments"), [
        ("has_model",                cpmeta!("hasModel"),              FieldType::String),
        ("has_serial_number",        cpmeta!("hasSerialNumber"),       FieldType::String),
        ("has_vendor",               cpmeta!("hasVendor"),             ref_to!(tbl!("ct_organizations"))),
        ("has_deployment",           cpmeta!("hasDeployment"),         FieldType::StringArray),
        ("has_etc_id",               cpmeta!("hasEtcId"),              FieldType::String),
        ("comment",                  rdfs!("comment"),                 FieldType::String),
        ("has_name",                 cpmeta!("hasName"),               FieldType::String),
        ("has_atc_id",               cpmeta!("hasAtcId"),              FieldType::String),
        ("has_instrument_owner",     cpmeta!("hasInstrumentOwner"),    ref_to!(tbl!("ct_organizations"))),
        ("has_instrument_component", cpmeta!("hasInstrumentComponent"), FieldType::StringArray),
        ("has_otc_id",               cpmeta!("hasOtcId"),              FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:Funding (115 instances)
    Funding, tbl!("ct_fundings"), [
        ("has_funder",    cpmeta!("hasFunder"),    ref_to!(tbl!("ct_funders"))),
        ("label",         rdfs!("label"),          FieldType::String),
        ("has_end_date",  cpmeta!("hasEndDate"),   FieldType::Date),
        ("has_start_date", cpmeta!("hasStartDate"), FieldType::Date),
        ("award_title",   cpmeta!("awardTitle"),   FieldType::String),
        ("award_number",  cpmeta!("awardNumber"),  FieldType::String),
        ("comment",       rdfs!("comment"),        FieldType::String),
        ("award_uri",     cpmeta!("awardURI"),     FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:WebpageElements (28 instances)
    WebpageElements, tbl!("ct_webpage_elements"), [
        ("has_linkbox",     cpmeta!("hasLinkbox"),    FieldType::StringArray),
        ("has_cover_image", cpmeta!("hasCoverImage"), FieldType::String),
        ("label",           rdfs!("label"),           FieldType::String),
        ("comment",         rdfs!("comment"),         FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:Membership (1,870 instances)
    Membership, tbl!("ct_memberships"), [
        ("label",                  rdfs!("label"),                    FieldType::StringArray),
        ("has_role",               cpmeta!("hasRole"),                ref_to!(tbl!("ct_roles"))),
        ("at_organization",        cpmeta!("atOrganization"),         ref_to!(tbl!("ct_organizations"), tbl!("ct_stations"))),
        ("has_start_time",         cpmeta!("hasStartTime"),           FieldType::DateTime),
        ("has_attribution_weight", cpmeta!("hasAttributionWeight"),   FieldType::Integer),
        ("has_end_time",           cpmeta!("hasEndTime"),             FieldType::DateTime),
        ("has_extra_role_info",    cpmeta!("hasExtraRoleInfo"),       FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:ThematicCenter (3 instances)
    ThematicCenter, tbl!("ct_thematic_centers"), [
        ("has_data_theme", cpmeta!("hasDataTheme"), ref_to!(tbl!("ct_data_themes"))),
        ("has_name",       cpmeta!("hasName"),      FieldType::String),
        ("label",          rdfs!("label"),          FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:ObjectFormat (22 instances)
    ObjectFormat, tbl!("ct_object_formats"), [
        ("label",              rdfs!("label"),             FieldType::String),
        ("has_good_flag_value", cpmeta!("hasGoodFlagValue"), FieldType::StringArray),
        ("comment",            rdfs!("comment"),           FieldType::String),
        ("see_also",           rdfs!("seeAlso"),           ref_to!(tbl!("ct_value_formats"))),
    ]
);

define_resource!(
    /// cpmeta:DatasetColumn (270 instances)
    DatasetColumn, tbl!("ct_dataset_columns"), [
        ("has_column_title",    cpmeta!("hasColumnTitle"),   FieldType::String),
        ("has_value_format",    cpmeta!("hasValueFormat"),   ref_to!(tbl!("ct_value_formats"))),
        ("has_value_type",      cpmeta!("hasValueType"),     ref_to!(tbl!("ct_value_types"))),
        ("label",               rdfs!("label"),              FieldType::String),
        ("is_optional_column",  cpmeta!("isOptionalColumn"), FieldType::Boolean),
        ("comment",             rdfs!("comment"),            FieldType::String),
        ("is_regex_column",     cpmeta!("isRegexColumn"),    FieldType::Boolean),
        ("is_quality_flag_for", cpmeta!("isQualityFlagFor"), FieldType::StringArray),
        ("see_also",            rdfs!("seeAlso"),            FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:DatasetVariable (76 instances)
    DatasetVariable, tbl!("ct_dataset_variables"), [
        ("has_value_type",      cpmeta!("hasValueType"),      ref_to!(tbl!("ct_value_types"))),
        ("has_variable_title",  cpmeta!("hasVariableTitle"),   FieldType::String),
        ("label",               rdfs!("label"),                FieldType::String),
        ("is_optional_variable", cpmeta!("isOptionalVariable"), FieldType::Boolean),
    ]
);

define_resource!(
    /// UNION: cpmeta:Station, AS, ES, OS, SailDrone, IngosStation, AtmoStation (623 instances)
    /// Discriminator column `station_type` IN ('station','as','es','os','saildrone','ingos','atmo')
    Station, tbl!("ct_stations"), [
        ("has_name",                    cpmeta!("hasName"),                    FieldType::String),
        ("country",                     cpmeta!("country"),                    FieldType::String),
        ("has_latitude",                cpmeta!("hasLatitude"),                FieldType::Double),
        ("has_longitude",               cpmeta!("hasLongitude"),               FieldType::Double),
        ("country_code",                cpmeta!("countryCode"),                FieldType::String),
        ("has_station_id",              cpmeta!("hasStationId"),               FieldType::String),
        ("has_elevation",               cpmeta!("hasElevation"),               FieldType::Double),
        ("has_responsible_organization", cpmeta!("hasResponsibleOrganization"), ref_to!(tbl!("ct_organizations"))),
        ("has_time_zone_offset",        cpmeta!("hasTimeZoneOffset"),          FieldType::Integer),
        ("label",                       rdfs!("label"),                        FieldType::String),
        ("comment",                     rdfs!("comment"),                      FieldType::StringArray),
        ("has_climate_zone",            cpmeta!("hasClimateZone"),             ref_to!(tbl!("ct_climate_zones"))),
        ("has_documentation_uri",       cpmeta!("hasDocumentationUri"),        FieldType::String),
        ("has_spatial_coverage",        cpmeta!("hasSpatialCoverage"),         ref_to!(tbl!("ct_spatial_coverages"))),
        ("theme",                       cpmeta!("theme"),                      FieldType::StringArray),
        ("has_atc_id",                  cpmeta!("hasAtcId"),                   FieldType::String),
        ("has_wigos_id",                cpmeta!("hasWigosId"),                 FieldType::String),
        ("has_station_class",           cpmeta!("hasStationClass"),            FieldType::String),
        ("has_documentation_object",    cpmeta!("hasDocumentationObject"),     FieldType::StringArray),
        ("has_depiction",               cpmeta!("hasDepiction"),               FieldType::StringArray),
        ("has_labeling_date",           cpmeta!("hasLabelingDate"),            FieldType::Date),
        ("contact_point",               cpmeta!("contactPoint"),               FieldType::StringArray),
        ("identifier",                  dcterms!("identifier"),                FieldType::String),
        ("is_part_of",                  dcterms!("isPartOf"),                  FieldType::String),
        ("spatial",                     dcterms!("spatial"),                   FieldType::StringArray),
        ("subject",                     dcterms!("subject"),                   FieldType::String),
        ("title",                       dcterms!("title"),                     FieldType::String),
        ("has_webpage_elements",        cpmeta!("hasWebpageElements"),         ref_to!(tbl!("ct_webpage_elements"))),
        ("has_etc_id",                  cpmeta!("hasEtcId"),                   FieldType::String),
        ("has_ecosystem_type",          cpmeta!("hasEcosystemType"),           ref_to!(tbl!("ct_ecosystem_types"))),
        ("has_mean_annual_precip",      cpmeta!("hasMeanAnnualPrecip"),        FieldType::Double),
        ("has_mean_annual_temp",        cpmeta!("hasMeanAnnualTemp"),          FieldType::Double),
        ("has_funding",                 cpmeta!("hasFunding"),                 FieldType::StringArray),
        ("description",                 dcterms!("description"),               FieldType::StringArray),
        ("has_mean_annual_radiation",   cpmeta!("hasMeanAnnualRadiation"),     FieldType::Double),
        ("has_associated_publication",  cpmeta!("hasAssociatedPublication"),   FieldType::StringArray),
        ("is_discontinued",             cpmeta!("isDiscontinued"),             FieldType::Boolean),
        ("has_otc_id",                  cpmeta!("hasOtcId"),                   FieldType::String),
        ("see_also",                    rdfs!("seeAlso"),                      FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:Person (1,146 instances)
    Person, tbl!("ct_persons"), [
        ("has_membership", cpmeta!("hasMembership"), FieldType::StringArray),
        ("has_first_name", cpmeta!("hasFirstName"),  FieldType::String),
        ("has_last_name",  cpmeta!("hasLastName"),   FieldType::String),
        ("has_email",      cpmeta!("hasEmail"),      FieldType::String),
        ("has_etc_id",     cpmeta!("hasEtcId"),      FieldType::String),
        ("has_orcid_id",   cpmeta!("hasOrcidId"),    FieldType::String),
        ("has_atc_id",     cpmeta!("hasAtcId"),      FieldType::String),
        ("has_otc_id",     cpmeta!("hasOtcId"),      FieldType::String),
        ("label",          rdfs!("label"),            FieldType::String),
        ("comment",        rdfs!("comment"),          FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:DataSubmission (2,344,302 instances)
    DataSubmission, tbl!("ct_data_submissions"), [
        ("ended_at_time",      prov!("endedAtTime"),      FieldType::DateTime),
        ("started_at_time",    prov!("startedAtTime"),     FieldType::DateTime),
        ("was_associated_with", prov!("wasAssociatedWith"), ref_to!(tbl!("ct_thematic_centers"))),
    ]
);

define_resource!(
    /// cpmeta:DataProduction (1,248,435 instances)
    DataProduction, tbl!("ct_data_productions"), [
        ("has_end_time",           cpmeta!("hasEndTime"),          FieldType::DateTime),
        ("was_performed_by",       cpmeta!("wasPerformedBy"),      ref_to!(tbl!("ct_thematic_centers"))),
        ("was_hosted_by",          cpmeta!("wasHostedBy"),         ref_to!(tbl!("ct_thematic_centers"))),
        ("was_participated_in_by", cpmeta!("wasParticipatedInBy"), FieldType::StringArray),
        ("comment",                rdfs!("comment"),               FieldType::String),
        ("see_also",               rdfs!("seeAlso"),               FieldType::String),
    ]
);

define_resource!(
    /// UNION: cpmeta:DatasetSpec, TabularDatasetSpec (45 instances)
    /// Discriminator column `dataset_type` IN ('dataset', 'tabular')
    DatasetSpec, tbl!("ct_dataset_specs"), [
        ("has_variable",            cpmeta!("hasVariable"),           FieldType::StringArray),
        ("label",                   rdfs!("label"),                   FieldType::String),
        ("has_temporal_resolution", cpmeta!("hasTemporalResolution"), FieldType::String),
        ("has_column",              cpmeta!("hasColumn"),             FieldType::StringArray),
        ("comment",                 rdfs!("comment"),                 FieldType::String),
    ]
);

define_resource!(
    /// cpmeta:DataAcquisition (2,341,317 instances)
    DataAcquisition, tbl!("ct_data_acquisitions"), [
        ("was_performed_with",  cpmeta!("wasPerformedWith"),  FieldType::StringArray),
        ("ended_at_time",       prov!("endedAtTime"),         FieldType::DateTime),
        ("started_at_time",     prov!("startedAtTime"),       FieldType::DateTime),
        ("was_associated_with", prov!("wasAssociatedWith"),    ref_to!(tbl!("ct_stations"))),
        ("has_sampling_height", cpmeta!("hasSamplingHeight"), FieldType::Double),
    ]
);

define_resource!(
    /// UNION: cpmeta:SimpleObjectSpec, DataObjectSpec (110 instances)
    /// Discriminator column `spec_type` IN ('simple', 'data')
    ObjectSpec, tbl!("ct_object_specs"), [
        ("contains_dataset",        cpmeta!("containsDataset"),       ref_to!(tbl!("ct_dataset_specs"))),
        ("has_associated_project",  cpmeta!("hasAssociatedProject"),  ref_to!(tbl!("ct_projects"))),
        ("has_data_level",          cpmeta!("hasDataLevel"),          FieldType::Integer),
        ("has_data_theme",          cpmeta!("hasDataTheme"),          ref_to!(tbl!("ct_data_themes"))),
        ("has_encoding",            cpmeta!("hasEncoding"),           ref_to!(tbl!("ct_object_encodings"))),
        ("has_format",              cpmeta!("hasFormat"),             ref_to!(tbl!("ct_object_formats"))),
        ("has_specific_dataset_type", cpmeta!("hasSpecificDatasetType"), ref_to!(tbl!("ct_specific_dataset_types"))),
        ("label",                   rdfs!("label"),                   FieldType::String),
        ("has_keywords",            cpmeta!("hasKeywords"),           FieldType::String),
        ("comment",                 rdfs!("comment"),                 FieldType::StringArray),
        ("has_documentation_object", cpmeta!("hasDocumentationObject"), FieldType::StringArray),
        ("implies_default_licence", cpmeta!("impliesDefaultLicence"), FieldType::String),
        ("see_also",                rdfs!("seeAlso"),                 FieldType::String),
    ]
);

define_resource!(
    /// UNION: cpmeta:DataObject, DocumentObject (2,344,302 instances)
    /// Discriminator column `object_type` IN ('data', 'document')
    StaticObject, tbl!("ct_static_objects"), [
        ("has_name",              cpmeta!("hasName"),             FieldType::String),
        ("has_object_spec",       cpmeta!("hasObjectSpec"),       ref_to!(tbl!("ct_object_specs"))),
        ("has_sha256sum",         cpmeta!("hasSha256sum"),        FieldType::String),
        ("has_size_in_bytes",     cpmeta!("hasSizeInBytes"),      FieldType::Integer),
        ("was_submitted_by",      cpmeta!("wasSubmittedBy"),      ref_to!(tbl!("ct_data_submissions"))),
        ("was_acquired_by",       cpmeta!("wasAcquiredBy"),       ref_to!(tbl!("ct_data_acquisitions"))),
        ("has_number_of_rows",    cpmeta!("hasNumberOfRows"),     FieldType::Integer),
        ("was_produced_by",       cpmeta!("wasProducedBy"),       ref_to!(tbl!("ct_data_productions"))),
        ("is_next_version_of",    cpmeta!("isNextVersionOf"),     ref_arr_to!(tbl!("ct_static_objects"))),
        ("has_actual_column_names", cpmeta!("hasActualColumnNames"), FieldType::String),
        ("had_primary_source",    prov!("hadPrimarySource"),      FieldType::StringArray),
        ("has_spatial_coverage",  cpmeta!("hasSpatialCoverage"),  ref_to!(tbl!("ct_spatial_coverages"))),
        ("has_actual_variable",   cpmeta!("hasActualVariable"),   FieldType::StringArray),
        ("has_doi",               cpmeta!("hasDoi"),              FieldType::String),
        ("has_keywords",          cpmeta!("hasKeywords"),         FieldType::String),
        ("contact_20_point",      cpmeta!("contactPoint"),        FieldType::String),
        ("contributor",           dcterms!("contributor"),         FieldType::String),
        ("measurement_20_method", cpmeta!("measurementMethod"),   FieldType::String),
        ("measurement_20_scale",  cpmeta!("measurementScale"),    FieldType::String),
        ("measurement_20_unit",   cpmeta!("measurementUnit"),     FieldType::String),
        ("observation_20_category", cpmeta!("observationCategory"), FieldType::String),
        ("parameter",             cpmeta!("parameter"),           FieldType::String),
        ("sampling_20_type",      cpmeta!("samplingType"),        FieldType::String),
        ("time_20_interval",      cpmeta!("timeInterval"),        FieldType::String),
        ("has_end_time",          cpmeta!("hasEndTime"),          FieldType::DateTime),
        ("has_start_time",        cpmeta!("hasStartTime"),        FieldType::DateTime),
        ("has_temporal_resolution", cpmeta!("hasTemporalResolution"), FieldType::String),
        ("description",           dcterms!("description"),        FieldType::String),
        ("title",                 dcterms!("title"),              FieldType::String),
        ("license",               dcterms!("license"),            FieldType::String),
        ("see_also",              rdfs!("seeAlso"),               FieldType::String),
        ("creator",               dcterms!("creator"),            FieldType::StringArray),
        ("deprecated_by_object",  cpmeta!("deprecatedByObject"),  ref_to!(tbl!("ct_static_objects"))),
        ("deprecated_by_collection", cpmeta!("deprecatedByCollection"), ref_to!(tbl!("ct_plain_collections"))),
    ]
);

define_resource!(
    /// cpmeta:Collection (778 instances)
    Collection, tbl!("ct_collections"), [
        ("has_part",             dcterms!("hasPart"),            FieldType::StringArray),
        ("creator",              dcterms!("creator"),            ref_to!(tbl!("ct_central_facilities"))),
        ("title",                dcterms!("title"),              FieldType::String),
        ("description",          dcterms!("description"),        FieldType::String),
        ("is_next_version_of",   cpmeta!("isNextVersionOf"),    ref_arr_to!(tbl!("ct_collections"))),
        ("has_doi",              cpmeta!("hasDoi"),              FieldType::String),
        ("has_spatial_coverage", cpmeta!("hasSpatialCoverage"),  ref_to!(tbl!("ct_spatial_coverages"))),
        ("see_also",             rdfs!("seeAlso"),               FieldType::String),
        ("deprecated_by",       cpmeta!("deprecatedBy"),         ref_to!(tbl!("ct_collections"))),
    ]
);

define_resource!(
    /// cpmeta:PlainCollection (50 instances)
    PlainCollection, tbl!("ct_plain_collections"), [
        ("has_part",           dcterms!("hasPart"),          FieldType::StringArray),
        ("is_next_version_of", cpmeta!("isNextVersionOf"), ref_to!(tbl!("ct_static_objects"))),
    ]
);

// ---------------------------------------------------------------------------
// Ontology class IRIs and rdfs:subClassOf relationships
// ---------------------------------------------------------------------------

/// All `(child, parent)` rdfs:subClassOf edges declared in `ontology/cpmeta.owl`.
/// Restrictions on anonymous classes are intentionally omitted: only edges
/// pointing at named superclasses are listed.
const CPMETA_SUBCLASS_EDGES: &[(&str, &str)] = &[
    (cpmeta!("AS"), cpmeta!("IcosStation")),
    (cpmeta!("AncillaryEntry"), cpmeta!("AncillaryDatum")),
    (cpmeta!("AncillaryValue"), cpmeta!("AncillaryDatum")),
    (cpmeta!("AtmoStation"), cpmeta!("Station")),
    (cpmeta!("CentralFacility"), cpmeta!("Organization")),
    (cpmeta!("CityMidLowCostStation"), cpmeta!("IcosCitiesStation")),
    (cpmeta!("Collection"), cpmeta!("PlainCollection")),
    (cpmeta!("DataAcquisition"), "http://www.w3.org/ns/prov#Activity"),
    (cpmeta!("DataObject"), cpmeta!("StaticObject")),
    (cpmeta!("DataObjectSpec"), cpmeta!("DataObjectSpecifyingThing")),
    (cpmeta!("DataProduction"), "http://www.w3.org/ns/prov#Activity"),
    (cpmeta!("DataSubmission"), "http://www.w3.org/ns/prov#Activity"),
    (cpmeta!("DataTheme"), cpmeta!("DataObjectSpecifyingThing")),
    (cpmeta!("DatasetColumn"), cpmeta!("DataObjectSpecifyingThing")),
    (cpmeta!("DatasetSpec"), cpmeta!("DataObjectSpecifyingThing")),
    (cpmeta!("DatasetVariable"), "http://www.w3.org/ns/sosa/ObservableProperty"),
    (cpmeta!("DocumentObject"), cpmeta!("StaticObject")),
    (cpmeta!("ES"), cpmeta!("IcosStation")),
    (cpmeta!("EtcNetwork"), cpmeta!("Network")),
    (cpmeta!("FluxnetStation"), cpmeta!("Station")),
    (cpmeta!("Funder"), cpmeta!("Organization")),
    (cpmeta!("IcosCitiesStation"), cpmeta!("Station")),
    (cpmeta!("IcosStation"), cpmeta!("Station")),
    (cpmeta!("IngosStation"), cpmeta!("Station")),
    (cpmeta!("Instrument"), "http://www.w3.org/ns/sosa/Sensor"),
    (cpmeta!("LinkBox"), cpmeta!("WebpageSpecifyingThing")),
    (cpmeta!("MunichMidLow"), cpmeta!("CityMidLowCostStation")),
    (cpmeta!("NeonStation"), cpmeta!("Station")),
    (cpmeta!("OS"), cpmeta!("IcosStation")),
    (cpmeta!("ObjectEncoding"), "http://purl.org/dc/terms/FileFormat"),
    (cpmeta!("ObjectFormat"), "http://purl.org/dc/terms/FileFormat"),
    (cpmeta!("Organization"), "http://www.w3.org/ns/prov#Agent"),
    (cpmeta!("ParisMidLow"), cpmeta!("CityMidLowCostStation")),
    (cpmeta!("Person"), "http://www.w3.org/ns/prov#Agent"),
    (cpmeta!("PlainCollection"), "http://www.w3.org/ns/prov#Entity"),
    (cpmeta!("Position"), cpmeta!("SpatialCoverage")),
    (cpmeta!("Project"), cpmeta!("DataObjectSpecifyingThing")),
    (cpmeta!("QuantityKind"), cpmeta!("DataObjectSpecifyingThing")),
    (cpmeta!("SailDrone"), cpmeta!("Station")),
    (cpmeta!("SimpleObjectSpec"), cpmeta!("DataObjectSpec")),
    (cpmeta!("Site"), "http://www.opengis.net/ont/geosparql#Feature"),
    (cpmeta!("SpatialCoverage"), "http://www.opengis.net/ont/geosparql#Geometry"),
    (cpmeta!("SpecificDatasetType"), cpmeta!("DataObjectSpecifyingThing")),
    (cpmeta!("StaticObject"), "http://www.w3.org/ns/prov#Entity"),
    (cpmeta!("Station"), cpmeta!("Organization")),
    (cpmeta!("StringVocabulary"), cpmeta!("ValueFormat")),
    (cpmeta!("TabularDatasetSpec"), cpmeta!("DatasetSpec")),
    (cpmeta!("ThematicCenter"), cpmeta!("CentralFacility")),
    (cpmeta!("ValueFormat"), cpmeta!("DataObjectSpecifyingThing")),
    (cpmeta!("ValueType"), "http://www.w3.org/2004/02/skos/core#Concept"),
    (cpmeta!("VariableInfo"), cpmeta!("DataObjectSpecifyingThing")),
    (cpmeta!("WdcggStation"), cpmeta!("Station")),
    (cpmeta!("WebpageElements"), cpmeta!("WebpageSpecifyingThing")),
    (cpmeta!("ZurichMidLow"), cpmeta!("CityMidLowCostStation")),
    ("http://www.opengis.net/ont/geosparql#Feature", "http://www.opengis.net/ont/geosparql#SpatialObject"),
    ("http://www.opengis.net/ont/geosparql#Geometry", "http://www.opengis.net/ont/geosparql#SpatialObject"),
    ("https://meta.fieldsites.se/ontologies/sites/Station", cpmeta!("Station")),
];

/// Class-IRI-to-table mappings derived from the OWL ontology. Each ontology
/// class IRI is mapped to the canonical SQL-table type IRI it should resolve
/// to. Most of these are inferred automatically by
/// `Schema::finalize_subclass_aliases`, but a handful are listed here for
/// classes whose names happen to coincide with a table type.
fn register_ontology_classes(schema: &mut Schema) {
    // The class IRIs that exactly correspond to single registered tables.
    schema.register_type_alias(
        Iri::new(cpmeta!("Station")),
        Iri::new(tbl!("ct_stations")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("StaticObject")),
        Iri::new(tbl!("ct_static_objects")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("Person")),
        Iri::new(tbl!("ct_persons")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("Organization")),
        Iri::new(tbl!("ct_organizations")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("CentralFacility")),
        Iri::new(tbl!("ct_central_facilities")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("Funder")),
        Iri::new(tbl!("ct_funders")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("ThematicCenter")),
        Iri::new(tbl!("ct_thematic_centers")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("Instrument")),
        Iri::new(tbl!("ct_instruments")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("Membership")),
        Iri::new(tbl!("ct_memberships")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("Funding")),
        Iri::new(tbl!("ct_fundings")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("Project")),
        Iri::new(tbl!("ct_projects")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("DataTheme")),
        Iri::new(tbl!("ct_data_themes")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("DataSubmission")),
        Iri::new(tbl!("ct_data_submissions")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("DataProduction")),
        Iri::new(tbl!("ct_data_productions")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("DataAcquisition")),
        Iri::new(tbl!("ct_data_acquisitions")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("ObjectEncoding")),
        Iri::new(tbl!("ct_object_encodings")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("ObjectFormat")),
        Iri::new(tbl!("ct_object_formats")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("ValueFormat")),
        Iri::new(tbl!("ct_value_formats")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("ValueType")),
        Iri::new(tbl!("ct_value_types")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("VariableInfo")),
        Iri::new(tbl!("ct_variable_infos")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("DatasetColumn")),
        Iri::new(tbl!("ct_dataset_columns")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("DatasetVariable")),
        Iri::new(tbl!("ct_dataset_variables")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("DatasetSpec")),
        Iri::new(tbl!("ct_dataset_specs")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("TabularDatasetSpec")),
        Iri::new(tbl!("ct_dataset_specs")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("EcosystemType")),
        Iri::new(tbl!("ct_ecosystem_types")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("ClimateZone")),
        Iri::new(tbl!("ct_climate_zones")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("QuantityKind")),
        Iri::new(tbl!("ct_quantity_kinds")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("Role")),
        Iri::new(tbl!("ct_roles")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("LinkBox")),
        Iri::new(tbl!("ct_link_boxes")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("WebpageElements")),
        Iri::new(tbl!("ct_webpage_elements")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("SpecificDatasetType")),
        Iri::new(tbl!("ct_specific_dataset_types")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("SpatialCoverage")),
        Iri::new(tbl!("ct_spatial_coverages")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("LatLonBox")),
        Iri::new(tbl!("ct_spatial_coverages")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("Position")),
        Iri::new(tbl!("ct_spatial_coverages")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("DataObject")),
        Iri::new(tbl!("ct_static_objects")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("DocumentObject")),
        Iri::new(tbl!("ct_static_objects")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("SimpleDataObject")),
        Iri::new(tbl!("ct_static_objects")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("SpatialDataObject")),
        Iri::new(tbl!("ct_static_objects")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("Collection")),
        Iri::new(tbl!("ct_collections")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("PlainCollection")),
        Iri::new(tbl!("ct_plain_collections")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("ObjectSpec")),
        Iri::new(tbl!("ct_object_specs")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("SimpleObjectSpec")),
        Iri::new(tbl!("ct_object_specs")),
    );
    schema.register_type_alias(
        Iri::new(cpmeta!("DataObjectSpec")),
        Iri::new(tbl!("ct_object_specs")),
    );

    // rdfs:subClassOf edges from the OWL ontology.
    for (child, parent) in CPMETA_SUBCLASS_EDGES {
        schema.register_subclass_of(Iri::new(*child), Iri::new(*parent));
    }
}

// ---------------------------------------------------------------------------
// Schema construction
// ---------------------------------------------------------------------------

pub fn cpmeta_schema() -> Schema {
    let mut schema = Schema::new();
    schema.register::<QuantityKind>();
    schema.register::<CentralFacility>();
    schema.register::<SpecificDatasetType>();
    schema.register::<Organization>();
    schema.register::<Funder>();
    schema.register::<EcosystemType>();
    schema.register::<SpatialCoverage>();
    schema.register::<Project>();
    schema.register::<VariableInfo>();
    schema.register::<LinkBox>();
    schema.register::<ClimateZone>();
    schema.register::<Role>();
    schema.register::<DataTheme>();
    schema.register::<ObjectEncoding>();
    schema.register::<ValueFormat>();
    schema.register::<ValueType>();
    schema.register::<Instrument>();
    schema.register::<Funding>();
    schema.register::<WebpageElements>();
    schema.register::<Membership>();
    schema.register::<ThematicCenter>();
    schema.register::<ObjectFormat>();
    schema.register::<DatasetColumn>();
    schema.register::<DatasetVariable>();
    schema.register::<Station>();
    schema.register::<Person>();
    schema.register::<DataSubmission>();
    schema.register::<DataProduction>();
    schema.register::<DatasetSpec>();
    schema.register::<DataAcquisition>();
    schema.register::<ObjectSpec>();
    schema.register::<StaticObject>();
    schema.register::<Collection>();
    schema.register::<PlainCollection>();

    register_ontology_classes(&mut schema);

    // Resolve every ontology class whose subclass closure unambiguously points
    // at one registered table. After this call, e.g. `cpmeta:IcosStation`
    // resolves to `ct_stations` because all of its descendants do.
    schema.finalize_subclass_aliases();

    // NOT EXISTS rewrites: `FILTER NOT EXISTS {[] cpmeta:isNextVersionOf ?x}`
    // becomes null-checks on the deprecation fields of ?x's type.
    schema.register_not_exists_rewrite(
        Iri::new(cpmeta!("isNextVersionOf")),
        Iri::new(tbl!("ct_static_objects")),
        vec!["deprecated_by_object", "deprecated_by_collection"],
    );
    schema.register_not_exists_rewrite(
        Iri::new(cpmeta!("isNextVersionOf")),
        Iri::new(tbl!("ct_collections")),
        vec!["deprecated_by"],
    );

    schema
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ct(s: &str) -> Iri {
        Iri::new(format!("http://meta.icos-cp.eu/tables/{}", s))
    }
    fn cp(s: &str) -> Iri {
        Iri::new(format!("http://meta.icos-cp.eu/ontologies/cpmeta/{}", s))
    }

    #[test]
    fn ontology_class_resolves_to_table() {
        let schema = cpmeta_schema();
        assert_eq!(schema.resolve_type(&cp("Station")), ct("ct_stations"));
        assert_eq!(schema.resolve_type(&cp("DataObject")), ct("ct_static_objects"));
        assert_eq!(schema.resolve_type(&cp("Collection")), ct("ct_collections"));
    }

    #[test]
    fn subclass_resolves_to_ancestor_table() {
        // Direct and transitive subclasses of `cpmeta:Station` all live in
        // `ct_stations`; the auto-alias step should pick that up.
        let schema = cpmeta_schema();
        for sub in &[
            "AS", "ES", "OS", "AtmoStation", "IcosStation", "IngosStation",
            "SailDrone", "FluxnetStation", "NeonStation", "WdcggStation",
            "IcosCitiesStation", "CityMidLowCostStation", "MunichMidLow",
            "ParisMidLow", "ZurichMidLow",
        ] {
            assert_eq!(
                schema.resolve_type(&cp(sub)),
                ct("ct_stations"),
                "expected {} to resolve to ct_stations",
                sub,
            );
        }
    }

    #[test]
    fn subclass_resolves_to_static_objects() {
        let schema = cpmeta_schema();
        for sub in &["DataObject", "DocumentObject", "SimpleDataObject", "SpatialDataObject"] {
            assert_eq!(
                schema.resolve_type(&cp(sub)),
                ct("ct_static_objects"),
                "expected {} to resolve to ct_static_objects",
                sub,
            );
        }
    }

    #[test]
    fn subclass_relationships_recorded() {
        let schema = cpmeta_schema();
        let parents = schema.direct_superclasses(&cp("AS"));
        assert!(parents.contains(&cp("IcosStation")));

        let ancestors = schema.ancestors_of(&cp("AS"));
        assert!(ancestors.contains(&cp("IcosStation")));
        assert!(ancestors.contains(&cp("Station")));
        assert!(ancestors.contains(&cp("Organization")));
    }

    #[test]
    fn ambiguous_superclass_is_not_aliased() {
        // `cpmeta:Organization` has subclasses spread across multiple tables
        // (ct_central_facilities, ct_funders, ct_organizations,
        // ct_thematic_centers, ct_stations). It must not auto-alias.
        let schema = cpmeta_schema();
        // Organization has its own table; alias should still be exact.
        assert_eq!(
            schema.resolve_type(&cp("Organization")),
            ct("ct_organizations"),
        );
        // But prov:Agent — a superclass of Organization AND Person —
        // straddles ct_organizations and ct_persons, so it stays unresolved.
        let agent = Iri::new("http://www.w3.org/ns/prov#Agent");
        assert_eq!(schema.resolve_type(&agent), agent);
    }
}

//! Schema definitions for the ICOS `stationentry` ontology
//! (`http://meta.icos-cp.eu/ontologies/stationentry/`).
//!
//! Generated from `ontology/stationEntry.owl` and mirroring the SQL tables in
//! `stationentry_tables.sql`. Three tables back six OWL classes:
//!
//! * `se_stations` — UNION of `Station`, `AS`, `ES`, `OS`
//! * `se_pis`      — `PI`
//! * `se_files`    — `File`
//!
//! Use [`register_stationentry`] to add these definitions to a [`Schema`].

use crate::rdf::Iri;
use crate::schema::{FieldType, Schema};

// ---------------------------------------------------------------------------
// Namespace helpers
// ---------------------------------------------------------------------------

macro_rules! tbl {
    ($t:literal) => {
        concat!("http://meta.icos-cp.eu/tables/", $t)
    };
}

macro_rules! sentry {
    ($p:literal) => {
        concat!("http://meta.icos-cp.eu/ontologies/stationentry/", $p)
    };
}

/// `http://meta.icos-cp.eu/files/` namespace — used by `File`'s `hasName`
/// and `hasType` properties (which are NOT in the stationentry namespace).
macro_rules! cpfiles {
    ($p:literal) => {
        concat!("http://meta.icos-cp.eu/files/", $p)
    };
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

define_resource!(
    /// UNION of `stationentry:Station`, `AS`, `ES`, `OS`.
    /// Discriminator column `station_type` IN ('station','as','es','os').
    SeStation, tbl!("se_stations"), [
        // -- Station ---------------------------------------------------------
        ("has_app_status_comment",       sentry!("hasAppStatusComment"),       FieldType::String),
        ("has_app_status_date",          sentry!("hasAppStatusDate"),          FieldType::DateTime),
        ("has_application_status",       sentry!("hasApplicationStatus"),      FieldType::String),
        ("has_country",                  sentry!("hasCountry"),                FieldType::String),
        ("has_description",              sentry!("hasDescription"),            FieldType::String),
        ("has_elevation_above_ground",   sentry!("hasElevationAboveGround"),   FieldType::String),
        ("has_elevation_above_sea",      sentry!("hasElevationAboveSea"),      FieldType::Float),
        ("has_funding_for_construction", sentry!("hasFundingForConstruction"), FieldType::String),
        ("has_funding_for_operation",    sentry!("hasFundingForOperation"),    FieldType::String),
        ("has_image_link",               sentry!("hasImageLink"),              FieldType::String),
        ("has_lat",                      sentry!("hasLat"),                    FieldType::String),
        ("has_lon",                      sentry!("hasLon"),                    FieldType::String),
        ("has_long_name",                sentry!("hasLongName"),               FieldType::String),
        ("has_operational_date_estimate", sentry!("hasOperationalDateEstimate"), FieldType::String),
        ("has_pre_icos_measurements",    sentry!("hasPreIcosMeasurements"),    FieldType::Boolean),
        ("has_production_counterpart",   sentry!("hasProductionCounterpart"),  FieldType::String),
        ("has_short_name",               sentry!("hasShortName"),              FieldType::String),
        ("has_site_type",                sentry!("hasSiteType"),               FieldType::String),
        ("has_station_class",            sentry!("hasStationClass"),           FieldType::String),
        ("has_station_kind",             sentry!("hasStationKind"),            FieldType::String),
        ("has_website",                  sentry!("hasWebsite"),                FieldType::String),
        ("is_already_operational",       sentry!("isAlreadyOperational"),      FieldType::Boolean),
        ("labeling_end_date",            sentry!("labelingEndDate"),           FieldType::DateTime),
        ("labeling_join_year",           sentry!("labelingJoinYear"),          FieldType::Integer),
        ("labeling_progress_date",       sentry!("labelingProgressDate"),      FieldType::String),
        ("step1_end_date",               sentry!("step1EndDate"),              FieldType::DateTime),
        ("step1_start_date",             sentry!("step1StartDate"),            FieldType::DateTime),
        ("step2_end_date",               sentry!("step2EndDate"),              FieldType::DateTime),
        ("step2_start_date",             sentry!("step2StartDate"),            FieldType::DateTime),
        ("has_associated_file",          sentry!("hasAssociatedFile"),         ref_to!(tbl!("se_files"))),
        ("has_deputy_pi",                sentry!("hasDeputyPi"),               ref_to!(tbl!("se_pis"))),
        ("has_pi",                       sentry!("hasPi"),                     ref_to!(tbl!("se_pis"))),

        // -- AS (Atmospheric Station) ---------------------------------------
        ("has_accessibility",                   sentry!("hasAccessibility"),               FieldType::String),
        ("has_address",                         sentry!("hasAddress"),                     FieldType::String),
        ("has_anthropogenics",                  sentry!("hasAnthropogenics"),              FieldType::String),
        ("has_atc_specific_value",              sentry!("hasAtcSpecificValue"),            FieldType::String),
        ("has_construction_end_date",           sentry!("hasConstructionEndDate"),         FieldType::String),
        ("has_construction_start_date",         sentry!("hasConstructionStartDate"),       FieldType::String),
        ("has_existing_infrastructure",         sentry!("hasExistingInfrastructure"),      FieldType::String),
        ("has_name_list_of_networks_it_belongs_to",
         sentry!("hasNameListOfNetworksItBelongsTo"),                                      FieldType::String),
        ("has_responsible_institution_name",    sentry!("hasResponsibleInstitutionName"),  FieldType::String),
        ("has_tc_id",                           sentry!("hasTcId"),                        FieldType::String),
        ("has_telecom",                         sentry!("hasTelecom"),                     FieldType::String),
        ("has_vegetation",                      sentry!("hasVegetation"),                  FieldType::String),

        // -- AS or OS (shared) ----------------------------------------------
        ("has_main_personnel_names_list",       sentry!("hasMainPersonnelNamesList"),      FieldType::String),

        // -- ES (Ecosystem Station) -----------------------------------------
        ("has_anemometer_direction",            sentry!("hasAnemometerDirection"),         FieldType::Integer),
        ("has_eddy_height",                     sentry!("hasEddyHeight"),                  FieldType::Float),
        ("has_etc_specific_value",              sentry!("hasEtcSpecificValue"),            FieldType::String),
        ("has_wind_data_in_european_database",  sentry!("hasWindDataInEuropeanDatabase"),  FieldType::Boolean),

        // -- OS (Ocean Station) ---------------------------------------------
        ("has_discrete_additional_info",                sentry!("hasDiscreteAdditionalInfo"),                FieldType::String),
        ("has_discrete_alkalinity_curve_fitting",       sentry!("hasDiscreteAlkalinityCurveFitting"),        FieldType::String),
        ("has_discrete_alkalinity_method_references",   sentry!("hasDiscreteAlkalinityMethodReferences"),    FieldType::String),
        ("has_discrete_alkalinity_other_titration",     sentry!("hasDiscreteAlkalinityOtherTitration"),      FieldType::String),
        ("has_discrete_alkalinity_titration_type",      sentry!("hasDiscreteAlkalinityTitrationType"),       FieldType::String),
        ("has_discrete_pco2_analysis",                  sentry!("hasDiscretePco2Analysis"),                  FieldType::String),
        ("has_discrete_pco2_analysis_method",           sentry!("hasDiscretePco2AnalysisMethod"),            FieldType::String),
        ("has_discrete_pco2_method_references",         sentry!("hasDiscretePco2MethodReferences"),          FieldType::String),
        ("has_discrete_ph_analysis_method",             sentry!("hasDiscretePhAnalysisMethod"),              FieldType::String),
        ("has_discrete_ph_method_references",           sentry!("hasDiscretePhMethodReferences"),            FieldType::String),
        ("has_discrete_ph_scale",                       sentry!("hasDiscretePhScale"),                       FieldType::String),
        ("has_discrete_tco2_analysis_method",           sentry!("hasDiscreteTco2AnalysisMethod"),            FieldType::String),
        ("has_discrete_tco2_method_references",         sentry!("hasDiscreteTco2MethodReferences"),          FieldType::String),
        ("has_discrete_tco2_standardization_technique", sentry!("hasDiscreteTco2StandardizationTechnique"),  FieldType::String),
        ("has_discrete_tco2_technique_description",     sentry!("hasDiscreteTco2TechniqueDescription"),      FieldType::String),
        ("has_easternmost_lon",                         sentry!("hasEasternmostLon"),                        FieldType::String),
        ("has_location_description",                    sentry!("hasLocationDescription"),                   FieldType::String),
        // Note: OWL spelling is "Nothernmost" (sic).
        ("has_nothernmost_lat",                         sentry!("hasNothernmostLat"),                        FieldType::String),
        ("has_nrt_data_delivery_method",                sentry!("hasNrtDataDeliveryMethod"),                 FieldType::String),
        ("has_nrt_data_update_frequency",               sentry!("hasNrtDataUpdateFrequency"),                FieldType::String),
        ("has_otc_specific_value",                      sentry!("hasOtcSpecificValue"),                      FieldType::String),
        ("has_platform_type",                           sentry!("hasPlatformType"),                          FieldType::String),
        ("has_southernmost_lat",                        sentry!("hasSouthernmostLat"),                       FieldType::String),
        ("has_spatial_reference",                       sentry!("hasSpatialReference"),                      FieldType::String),
        ("has_type_of_sampling",                        sentry!("hasTypeOfSampling"),                        FieldType::String),
        ("has_underway_additional_info",                sentry!("hasUnderwayAdditionalInfo"),                FieldType::String),
        ("has_underway_co2_sensor_manufacturer",        sentry!("hasUnderwayCo2SensorManufacturer"),         FieldType::String),
        ("has_underway_co2_sensor_model",               sentry!("hasUnderwayCo2SensorModel"),                FieldType::String),
        ("has_underway_equilibrator_type",              sentry!("hasUnderwayEquilibratorType"),              FieldType::String),
        ("has_underway_method_references",              sentry!("hasUnderwayMethodReferences"),              FieldType::String),
        ("has_underway_other_sensor_manufacturer",      sentry!("hasUnderwayOtherSensorManufacturer"),       FieldType::String),
        ("has_underway_other_sensor_model",             sentry!("hasUnderwayOtherSensorModel"),              FieldType::String),
        ("has_vessel_owner",                            sentry!("hasVesselOwner"),                           FieldType::String),
        ("has_westernmost_lon",                         sentry!("hasWesternmostLon"),                        FieldType::String),
    ]
);

define_resource!(
    /// `stationentry:PI` — Principal Investigator.
    SePi, tbl!("se_pis"), [
        ("has_affiliation", sentry!("hasAffiliation"), FieldType::String),
        ("has_email",       sentry!("hasEmail"),       FieldType::String),
        ("has_first_name",  sentry!("hasFirstName"),   FieldType::String),
        ("has_last_name",   sentry!("hasLastName"),    FieldType::String),
        ("has_phone",       sentry!("hasPhone"),       FieldType::String),
    ]
);

define_resource!(
    /// `stationentry:File` — `hasName`/`hasType` use the `files/` namespace.
    SeFile, tbl!("se_files"), [
        ("has_name", cpfiles!("hasName"), FieldType::String),
        ("has_type", cpfiles!("hasType"), FieldType::String),
    ]
);

// ---------------------------------------------------------------------------
// Ontology classes and rdfs:subClassOf relationships
// ---------------------------------------------------------------------------

/// All `(child, parent)` rdfs:subClassOf edges declared in
/// `ontology/stationEntry.owl`. AS, ES, OS are direct subclasses of Station.
const STATIONENTRY_SUBCLASS_EDGES: &[(&str, &str)] = &[
    (sentry!("AS"), sentry!("Station")),
    (sentry!("ES"), sentry!("Station")),
    (sentry!("OS"), sentry!("Station")),
];

/// Register the stationentry tables, type aliases for ontology classes,
/// and rdfs:subClassOf edges into the given [`Schema`].
///
/// Call [`Schema::finalize_subclass_aliases`] afterwards (typically once
/// for the entire schema) so AS/ES/OS resolve to `se_stations`.
pub fn register_stationentry(schema: &mut Schema) {
    schema.register::<SeStation>();
    schema.register::<SePi>();
    schema.register::<SeFile>();

    // Direct ontology-class-to-table aliases.
    schema.register_type_alias(
        Iri::new(sentry!("Station")),
        Iri::new(tbl!("se_stations")),
    );
    schema.register_type_alias(
        Iri::new(sentry!("PI")),
        Iri::new(tbl!("se_pis")),
    );
    schema.register_type_alias(
        Iri::new(sentry!("File")),
        Iri::new(tbl!("se_files")),
    );

    for (child, parent) in STATIONENTRY_SUBCLASS_EDGES {
        schema.register_subclass_of(Iri::new(*child), Iri::new(*parent));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Iri {
        Iri::new(format!("http://meta.icos-cp.eu/tables/{}", s))
    }
    fn s(s: &str) -> Iri {
        Iri::new(format!("http://meta.icos-cp.eu/ontologies/stationentry/{}", s))
    }

    fn build() -> Schema {
        let mut schema = Schema::new();
        register_stationentry(&mut schema);
        schema.finalize_subclass_aliases();
        schema
    }

    #[test]
    fn ontology_classes_resolve_to_tables() {
        let schema = build();
        assert_eq!(schema.resolve_type(&s("Station")), t("se_stations"));
        assert_eq!(schema.resolve_type(&s("PI")),      t("se_pis"));
        assert_eq!(schema.resolve_type(&s("File")),    t("se_files"));
    }

    #[test]
    fn subclasses_resolve_to_se_stations() {
        let schema = build();
        for sub in &["AS", "ES", "OS"] {
            assert_eq!(
                schema.resolve_type(&s(sub)),
                t("se_stations"),
                "expected stationentry:{} to resolve to se_stations",
                sub,
            );
        }
    }

    #[test]
    fn subclass_relationships_recorded() {
        let schema = build();
        for sub in &["AS", "ES", "OS"] {
            let parents = schema.direct_superclasses(&s(sub));
            assert!(
                parents.contains(&s("Station")),
                "{} should be a subclass of Station",
                sub,
            );
        }
    }

    #[test]
    fn predicates_are_known() {
        let schema = build();
        // Sample one predicate from each domain class.
        for pred in &[
            sentry!("hasShortName"),       // Station
            sentry!("hasAccessibility"),   // AS
            sentry!("hasEddyHeight"),      // ES
            sentry!("hasPlatformType"),    // OS
            sentry!("hasFirstName"),       // PI
            cpfiles!("hasName"),           // File (different namespace!)
            sentry!("hasPi"),              // ObjectProperty
        ] {
            assert!(
                schema.is_known_predicate(&Iri::new(*pred)),
                "expected predicate {} to be known",
                pred,
            );
        }
    }
}

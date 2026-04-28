BEGIN;
-- SQL schema for the ICOS stationentry ontology
-- (http://meta.icos-cp.eu/ontologies/stationentry/).
--
-- Source: ontology/stationEntry.owl
--
-- Three tables back the six OWL classes:
--   se_stations  — UNION of stationentry:Station, AS, ES, OS
--   se_pis       — stationentry:PI
--   se_files     — stationentry:File
--
-- Columns are derived from owl:DatatypeProperty / owl:ObjectProperty
-- declarations. Properties whose domain is one of {Station, AS, ES, OS}
-- all live on se_stations; the discriminator column `station_type`
-- distinguishes the four sub-tables.

DROP TABLE IF EXISTS se_stations CASCADE;
DROP TABLE IF EXISTS se_pis CASCADE;
DROP TABLE IF EXISTS se_files CASCADE;

-- ======================================================================
-- se_stations  (UNION of Station, AS, ES, OS)
-- ======================================================================

CREATE UNLOGGED TABLE IF NOT EXISTS se_stations (
    id TEXT PRIMARY KEY,
    rdf_subject TEXT NOT NULL UNIQUE,
    prefix TEXT NOT NULL,
    station_type TEXT NOT NULL CHECK (station_type IN ('station', 'as', 'es', 'os')),

    -- Station (parent class)
    has_app_status_comment TEXT,
    has_app_status_date TIMESTAMP WITH TIME ZONE,
    has_application_status TEXT,
    has_country TEXT,
    has_description TEXT,
    has_elevation_above_ground TEXT,
    has_elevation_above_sea DOUBLE PRECISION,
    has_funding_for_construction TEXT,
    has_funding_for_operation TEXT,
    has_image_link TEXT,
    has_lat TEXT,
    has_lon TEXT,
    has_long_name TEXT,
    has_operational_date_estimate TEXT,
    has_pre_icos_measurements BOOLEAN,
    has_production_counterpart TEXT,
    has_short_name TEXT,
    has_site_type TEXT,
    has_station_class TEXT,
    has_station_kind TEXT,
    has_website TEXT,
    is_already_operational BOOLEAN,
    labeling_end_date TIMESTAMP WITH TIME ZONE,
    labeling_join_year INTEGER,
    labeling_progress_date TEXT,
    step1_end_date TIMESTAMP WITH TIME ZONE,
    step1_start_date TIMESTAMP WITH TIME ZONE,
    step2_end_date TIMESTAMP WITH TIME ZONE,
    step2_start_date TIMESTAMP WITH TIME ZONE,
    has_associated_file TEXT,
    has_deputy_pi TEXT,
    has_pi TEXT,

    -- AS (Atmospheric Station)
    has_accessibility TEXT,
    has_address TEXT,
    has_anthropogenics TEXT,
    has_atc_specific_value TEXT,
    has_construction_end_date TEXT,
    has_construction_start_date TEXT,
    has_existing_infrastructure TEXT,
    has_name_list_of_networks_it_belongs_to TEXT,
    has_responsible_institution_name TEXT,
    has_tc_id TEXT,
    has_telecom TEXT,
    has_vegetation TEXT,

    -- AS or OS (shared)
    has_main_personnel_names_list TEXT,

    -- ES (Ecosystem Station)
    has_anemometer_direction INTEGER,
    has_eddy_height DOUBLE PRECISION,
    has_etc_specific_value TEXT,
    has_wind_data_in_european_database BOOLEAN,

    -- OS (Ocean Station)
    has_discrete_additional_info TEXT,
    has_discrete_alkalinity_curve_fitting TEXT,
    has_discrete_alkalinity_method_references TEXT,
    has_discrete_alkalinity_other_titration TEXT,
    has_discrete_alkalinity_titration_type TEXT,
    has_discrete_pco2_analysis TEXT,
    has_discrete_pco2_analysis_method TEXT,
    has_discrete_pco2_method_references TEXT,
    has_discrete_ph_analysis_method TEXT,
    has_discrete_ph_method_references TEXT,
    has_discrete_ph_scale TEXT,
    has_discrete_tco2_analysis_method TEXT,
    has_discrete_tco2_method_references TEXT,
    has_discrete_tco2_standardization_technique TEXT,
    has_discrete_tco2_technique_description TEXT,
    has_easternmost_lon TEXT,
    has_location_description TEXT,
    has_nothernmost_lat TEXT,
    has_nrt_data_delivery_method TEXT,
    has_nrt_data_update_frequency TEXT,
    has_otc_specific_value TEXT,
    has_platform_type TEXT,
    has_southernmost_lat TEXT,
    has_spatial_reference TEXT,
    has_type_of_sampling TEXT,
    has_underway_additional_info TEXT,
    has_underway_co2_sensor_manufacturer TEXT,
    has_underway_co2_sensor_model TEXT,
    has_underway_equilibrator_type TEXT,
    has_underway_method_references TEXT,
    has_underway_other_sensor_manufacturer TEXT,
    has_underway_other_sensor_model TEXT,
    has_vessel_owner TEXT,
    has_westernmost_lon TEXT,

    CHECK (prefix || id = rdf_subject)
);

-- ======================================================================
-- se_pis  (stationentry:PI)
-- ======================================================================

CREATE UNLOGGED TABLE IF NOT EXISTS se_pis (
    id TEXT PRIMARY KEY,
    rdf_subject TEXT NOT NULL UNIQUE,
    prefix TEXT NOT NULL,
    has_affiliation TEXT,
    has_email TEXT,
    has_first_name TEXT,
    has_last_name TEXT,
    has_phone TEXT,
    CHECK (prefix || id = rdf_subject)
);

-- ======================================================================
-- se_files  (stationentry:File)
-- ======================================================================

CREATE UNLOGGED TABLE IF NOT EXISTS se_files (
    id TEXT PRIMARY KEY,
    rdf_subject TEXT NOT NULL UNIQUE,
    prefix TEXT NOT NULL,
    -- Note: hasName / hasType use the http://meta.icos-cp.eu/files/ namespace,
    -- not the stationentry/ namespace.
    has_name TEXT,
    has_type TEXT,
    CHECK (prefix || id = rdf_subject)
);

COMMIT;

-- ======================================================================
-- Foreign keys
-- ======================================================================

BEGIN;

ALTER TABLE se_stations ADD CONSTRAINT fk_se_stations_has_pi
    FOREIGN KEY (has_pi) REFERENCES se_pis(id);
ALTER TABLE se_stations ADD CONSTRAINT fk_se_stations_has_deputy_pi
    FOREIGN KEY (has_deputy_pi) REFERENCES se_pis(id);
ALTER TABLE se_stations ADD CONSTRAINT fk_se_stations_has_associated_file
    FOREIGN KEY (has_associated_file) REFERENCES se_files(id);

COMMIT;

-- ======================================================================
-- Indexes
-- ======================================================================

CREATE INDEX IF NOT EXISTS idx_se_stations_has_short_name ON se_stations(has_short_name);
CREATE INDEX IF NOT EXISTS idx_se_stations_has_long_name ON se_stations(has_long_name);
CREATE INDEX IF NOT EXISTS idx_se_stations_station_type ON se_stations(station_type);
CREATE INDEX IF NOT EXISTS idx_se_stations_has_country ON se_stations(has_country);
CREATE INDEX IF NOT EXISTS idx_se_stations_has_application_status ON se_stations(has_application_status);
CREATE INDEX IF NOT EXISTS idx_se_stations_has_pi ON se_stations(has_pi);
CREATE INDEX IF NOT EXISTS idx_se_stations_has_deputy_pi ON se_stations(has_deputy_pi);
CREATE INDEX IF NOT EXISTS idx_se_stations_has_tc_id ON se_stations(has_tc_id);

CREATE INDEX IF NOT EXISTS idx_se_pis_has_first_name ON se_pis(has_first_name);
CREATE INDEX IF NOT EXISTS idx_se_pis_has_last_name ON se_pis(has_last_name);
CREATE INDEX IF NOT EXISTS idx_se_pis_has_email ON se_pis(has_email);

CREATE INDEX IF NOT EXISTS idx_se_files_has_name ON se_files(has_name);
CREATE INDEX IF NOT EXISTS idx_se_files_has_type ON se_files(has_type);

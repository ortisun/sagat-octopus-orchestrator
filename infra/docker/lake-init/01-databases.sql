-- Databases for the lake fixtures (feature 035). The `lake` role owns every one of them.
-- The JDBC/SQL Iceberg catalog uses POSTGRES_DB (`iceberg_catalog`); Lakekeeper keeps its own state.
-- Disposable fixture data only: this file never touches the platform database.
CREATE DATABASE lakekeeper OWNER lake;

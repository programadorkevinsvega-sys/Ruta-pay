\getenv exporter_password POSTGRES_EXPORTER_PASSWORD
\getenv migrator_password PLATFORM_MIGRATOR_PASSWORD
\getenv runtime_password PLATFORM_RUNTIME_PASSWORD
CREATE ROLE postgres_exporter LOGIN PASSWORD :'exporter_password';
GRANT pg_monitor TO postgres_exporter;
CREATE ROLE platform_migrator LOGIN PASSWORD :'migrator_password';
CREATE ROLE platform_runtime LOGIN PASSWORD :'runtime_password';
GRANT CONNECT, CREATE ON DATABASE payvia TO platform_migrator;
GRANT CONNECT ON DATABASE payvia TO platform_runtime;

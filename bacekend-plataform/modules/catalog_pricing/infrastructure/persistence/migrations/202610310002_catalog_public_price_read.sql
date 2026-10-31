DROP POLICY catalog_prices_operator_read ON catalog_pricing.price_versions;
CREATE POLICY catalog_prices_public_read ON catalog_pricing.price_versions FOR SELECT USING (true);

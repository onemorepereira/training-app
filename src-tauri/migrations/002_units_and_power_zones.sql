-- Unit system preference
ALTER TABLE user_config ADD COLUMN units TEXT NOT NULL DEFAULT 'metric';

-- Power zones as % of FTP (Coggan 7-zone: upper bound of Z1-Z6, Z7 is everything above Z6)
ALTER TABLE user_config ADD COLUMN power_zone_1 INTEGER NOT NULL DEFAULT 55;
ALTER TABLE user_config ADD COLUMN power_zone_2 INTEGER NOT NULL DEFAULT 75;
ALTER TABLE user_config ADD COLUMN power_zone_3 INTEGER NOT NULL DEFAULT 90;
ALTER TABLE user_config ADD COLUMN power_zone_4 INTEGER NOT NULL DEFAULT 105;
ALTER TABLE user_config ADD COLUMN power_zone_5 INTEGER NOT NULL DEFAULT 120;
ALTER TABLE user_config ADD COLUMN power_zone_6 INTEGER NOT NULL DEFAULT 150;

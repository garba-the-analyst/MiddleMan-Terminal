INSERT INTO price_catalogue (brand, country, card_format, rate_per_dollar) VALUES
    ('STEAM',       'US', 'PHYSICAL', 1450),
    ('STEAM',       'US', 'ECODE',    1430),
    ('APPLE',       'US', 'PHYSICAL', 1500),
    ('APPLE',       'UK', 'ECODE',    1470),
    ('AMAZON',      'US', 'PHYSICAL', 1420),
    ('RAZER_GOLD',  'US', 'ECODE',    1380),
    ('GOOGLE_PLAY', 'US', 'ECODE',    1360)
ON CONFLICT DO NOTHING;

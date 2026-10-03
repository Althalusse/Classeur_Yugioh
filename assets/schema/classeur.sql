-- DDL d'une base de classeur — EXTRAIT de RA05.db (V1.0.3, 694 cartes).
-- NE PAS EDITER A LA MAIN.
-- Le <CODE> du fichier est le code du set ; le schema est identique pour tous.

CREATE TABLE cards (
                card_uuid        TEXT,
                card_image_uuid  TEXT,
                card_image_id    INTEGER,
                set_code         TEXT,
                rarity           TEXT,
                rarity_code      TEXT    DEFAULT '',
                set_name         TEXT,
                name             TEXT,
                name_fr          TEXT    DEFAULT '',
                card_image_url   TEXT,
                card_image_small TEXT    DEFAULT '',
                sort_order       INTEGER DEFAULT 0,
                card_type        TEXT    DEFAULT '',
                atk              INTEGER,
                def_val          INTEGER,
                level            INTEGER,
                attribute        TEXT    DEFAULT '',
                race             TEXT    DEFAULT '',
                possessed        INTEGER DEFAULT 0,
                quantite         INTEGER DEFAULT 0,
                qualite          TEXT    DEFAULT NULL,
                edition          TEXT    DEFAULT NULL,
                extended_art     INTEGER DEFAULT 0,
                is_custom        INTEGER DEFAULT 0
            );

CREATE TABLE meta (
            key   TEXT PRIMARY KEY,
            value TEXT
        );

CREATE INDEX idx_card_id ON cards(card_image_id);

CREATE INDEX idx_sort    ON cards(sort_order);


-- DDL de cardinfo.db — EXTRAIT de la base reelle produite par la V1.0.4 Python.
-- NE PAS EDITER A LA MAIN. Regenerer avec : sqlite3 cardinfo.db .schema
-- Source : Projet Python/V1.0.4/bdd/cardinfo.db (2026-08-25)

CREATE TABLE anomalies (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                name                TEXT    NOT NULL,
                art_a_image_uuid    TEXT    NOT NULL,
                art_b_image_uuid    TEXT    NOT NULL,
                art_index           INTEGER NOT NULL,
                set_code_prefix     TEXT    NOT NULL,
                missing_set_code    TEXT    NOT NULL,
                missing_set_rarity  TEXT    NOT NULL,
                image_url           TEXT,
                image_url_small     TEXT,
                image_id            INTEGER,
                corrige             INTEGER DEFAULT 0,
                UNIQUE(art_b_image_uuid, missing_set_code, missing_set_rarity)
            );

CREATE TABLE card_images (
            uuid                TEXT PRIMARY KEY,
            card_uuid           TEXT NOT NULL,
            ygoprodeck_image_id INTEGER,
            art_url             TEXT,
            card_url            TEXT,
            FOREIGN KEY (card_uuid) REFERENCES cards(uuid)
        );

CREATE TABLE card_images_externes (
    uuid       TEXT PRIMARY KEY,
    card_name  TEXT NOT NULL,
    set_prefix TEXT NOT NULL,
    langue     TEXT DEFAULT '',
    rarete     TEXT DEFAULT '',
    edition    TEXT DEFAULT '',
    variante   TEXT DEFAULT '',
    image_id   INTEGER,
    art_url    TEXT,
    card_url   TEXT,
    fichier    TEXT,
    source     TEXT DEFAULT 'yugipedia',
    date_maj   TEXT
);

CREATE TABLE card_texts (
            card_uuid TEXT NOT NULL,
            language  TEXT NOT NULL,
            name      TEXT,
            effect    TEXT,
            PRIMARY KEY (card_uuid, language),
            FOREIGN KEY (card_uuid) REFERENCES cards(uuid)
        );

CREATE TABLE cards (
            uuid              TEXT PRIMARY KEY,
            ygoprodeck_id     INTEGER,
            card_type         TEXT,
            subcategory       TEXT,
            frame_type        TEXT,
            atk               INTEGER,
            def               INTEGER,
            level             INTEGER,
            attribute         TEXT,
            race              TEXT,
            banlist_tcg       TEXT,
            banlist_ocg       TEXT,
            name_fr_confirmed INTEGER DEFAULT 0
        );

CREATE TABLE cards_missing_fr (
            id        INTEGER PRIMARY KEY,
            card_uuid TEXT,
            name      TEXT NOT NULL,
            card_type TEXT,
            image_url TEXT
        );

CREATE TABLE cards_overrides (
            override_id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            base_card_id                 INTEGER,
            name                         TEXT NOT NULL,
            card_images_id               INTEGER,
            card_images_image_url        TEXT,
            card_images_image_url_small  TEXT,
            card_sets_set_name           TEXT,
            card_sets_set_code           TEXT NOT NULL,
            card_sets_set_rarity         TEXT NOT NULL,
            card_sets_set_rarity_code    TEXT,
            reason                       TEXT,
            created_at                   TEXT DEFAULT (datetime('now')),
            UNIQUE(base_card_id, card_sets_set_code, card_sets_set_rarity)
        );

CREATE TABLE overframe_sync (
            set_prefix TEXT PRIMARY KEY,
            revid      TEXT,
            synced_at  TEXT
        );

CREATE TABLE set_locales (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            set_uuid          TEXT NOT NULL,
            language          TEXT NOT NULL,
            prefix            TEXT,
            release_date      TEXT,
            booster_image_url TEXT,
            FOREIGN KEY (set_uuid) REFERENCES sets(uuid),
            UNIQUE (set_uuid, language)
        );

CREATE TABLE set_prints (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            set_uuid        TEXT NOT NULL,
            set_locale_id   INTEGER NOT NULL,
            card_uuid       TEXT NOT NULL,
            card_image_uuid TEXT,
            set_code        TEXT,
            rarity          TEXT,
            edition         TEXT,
            qty             INTEGER DEFAULT 1,
            print_image_url TEXT,
            extended_art    INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (set_uuid)        REFERENCES sets(uuid),
            FOREIGN KEY (set_locale_id)   REFERENCES set_locales(id),
            FOREIGN KEY (card_uuid)       REFERENCES cards(uuid),
            FOREIGN KEY (card_image_uuid) REFERENCES card_images(uuid)
        );

CREATE TABLE sets (
            uuid     TEXT PRIMARY KEY,
            name_en  TEXT,
            name_fr  TEXT,
            name_de  TEXT,
            name_it  TEXT,
            name_es  TEXT,
            name_ja  TEXT,
            name_ko  TEXT
        );

CREATE INDEX idx_card_images_card_uuid  ON card_images(card_uuid);

CREATE INDEX idx_card_images_ygo_id     ON card_images(ygoprodeck_image_id);

CREATE INDEX idx_card_texts_card_lang   ON card_texts(card_uuid, language);

CREATE INDEX idx_cards_ygoprodeck_id    ON cards(ygoprodeck_id);

CREATE INDEX idx_cie_name_set ON card_images_externes(card_name, set_prefix);

CREATE INDEX idx_set_locales_set_uuid   ON set_locales(set_uuid);

CREATE INDEX idx_set_prints_card_uuid   ON set_prints(card_uuid);

CREATE INDEX idx_set_prints_rarity      ON set_prints(rarity);

CREATE INDEX idx_set_prints_set_code    ON set_prints(set_code);

CREATE INDEX idx_set_prints_set_locale  ON set_prints(set_locale_id);


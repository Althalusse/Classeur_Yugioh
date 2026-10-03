// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! YGOPRODeck — statistiques et résolution des artworks.
//!
//! Portage de `module/ygoprodeck_enricher.py`.
//!
//! # Deux rôles distincts
//!
//! **1. Enrichissement.** YGOJSON connaît les cartes, leurs textes et leurs
//! illustrations, mais pas leurs statistiques. YGOPRODeck les fournit, indexées
//! par « password » Konami. [`enrichir_cartes`] les recopie dans les lignes de
//! `cards`.
//!
//! **2. Résolution et expansion des artworks.** C'est la partie subtile, et
//! elle règle un vrai problème de modélisation :
//!
//! - les UUID de tirage de YGOJSON (`contents[].cards[].id`) **ne sont pas** les
//!   UUID d'illustration de `card_images` — il faut les relier par les
//!   passwords ;
//! - quand YGOPRODeck confirme **plusieurs** passwords pour un même
//!   `(set_code, rareté)`, c'est qu'il existe plusieurs illustrations pour ce
//!   tirage : une ligne de `set_prints` est alors générée **par illustration**.
//!
//! Sans cette passe, une carte à deux artworks n'apparaît qu'une fois dans le
//! classeur, et c'est le mauvais artwork une fois sur deux.
//!
//! # Dégradation
//!
//! YGOPRODeck indisponible n'est pas fatal : les statistiques manquent, les
//! tirages restent tels quels, l'initialisation continue. Le Python le signale
//! par une liste vide ; ici, [`resoudre_et_etendre_artworks`] renvoie les
//! tirages inchangés quand le catalogue est vide, et l'appelant décide.

use std::collections::HashMap;

use serde::Deserialize;
use ygo_core::modele::{LigneCarte, LigneImage, TirageBrut};

use crate::error::Result;
use crate::http::ClientHttp;

/// Catalogue complet de l'API YGOPRODeck, alias compris.
///
/// `includeAliased=true` est indispensable : sans lui, les artworks alternatifs
/// — qui sont précisément l'objet de l'expansion — n'apparaissent pas.
pub const YGOPRODECK_URL: &str =
    "https://db.ygoprodeck.com/api/v7/cardinfo.php?includeAliased=true";

/// Réponse de `cardinfo.php`.
#[derive(Debug, Clone, Deserialize)]
pub struct ReponseCatalogue {
    /// Les cartes.
    #[serde(default)]
    pub data: Vec<CarteYgoprodeck>,
}

/// Une carte du catalogue YGOPRODeck.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CarteYgoprodeck {
    /// « Password » Konami.
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    /// Type de cadre.
    #[serde(rename = "frameType", default)]
    pub frame_type: Option<String>,
    /// Attaque.
    #[serde(default)]
    pub atk: Option<serde_json::Value>,
    /// Défense.
    #[serde(default)]
    pub def: Option<serde_json::Value>,
    /// Niveau / rang.
    #[serde(default)]
    pub level: Option<serde_json::Value>,
    /// Attribut.
    #[serde(default)]
    pub attribute: Option<String>,
    /// Type de monstre.
    #[serde(default)]
    pub race: Option<String>,
    /// Statuts de bannissement.
    #[serde(default)]
    pub banlist_info: Option<InfoBanlist>,
    /// Tirages connus de cette carte.
    #[serde(default)]
    pub card_sets: Vec<TirageYgoprodeck>,
}

/// Statuts de bannissement d'une carte.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct InfoBanlist {
    /// Statut TCG.
    #[serde(default)]
    pub ban_tcg: Option<String>,
    /// Statut OCG.
    #[serde(default)]
    pub ban_ocg: Option<String>,
}

/// Un tirage tel que YGOPRODeck le décrit.
#[derive(Debug, Clone, Deserialize)]
pub struct TirageYgoprodeck {
    /// Code de set (`RA05-EN134`).
    #[serde(default)]
    pub set_code: String,
    /// Rareté, dans le vocabulaire YGOPRODeck.
    #[serde(default)]
    pub set_rarity: String,
}

/// Convertit une valeur JSON en entier, en acceptant nombre **et** chaîne.
///
/// L'API renvoie parfois `"?"` pour l'attaque d'un monstre à valeur variable :
/// le Python l'écarte par son `except (ValueError, TypeError)`, on fait pareil.
fn vers_i64(v: &serde_json::Value) -> Option<i64> {
    match v {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// Télécharge le catalogue complet.
///
/// Retourne `Ok(vec![])` si la source répond mal — l'appelant traite ce cas
/// comme une dégradation, pas comme un échec.
pub async fn telecharger_catalogue(client: &ClientHttp) -> Result<Vec<CarteYgoprodeck>> {
    let reponse: ReponseCatalogue = client.get_json(YGOPRODECK_URL).await?;
    tracing::info!(entrees = reponse.data.len(), "catalogue YGOPRODeck reçu");
    Ok(reponse.data)
}

/// Index `password → carte`, pour l'enrichissement.
///
/// Portage de `build_ygoprodeck_index`.
pub fn indexer_par_password(cartes: &[CarteYgoprodeck]) -> HashMap<i64, &CarteYgoprodeck> {
    let mut index = HashMap::with_capacity(cartes.len());
    for carte in cartes {
        if let Some(pw) = carte.id.as_ref().and_then(vers_i64) {
            index.entry(pw).or_insert(carte);
        }
    }
    index
}

/// Complète les statistiques des lignes de `cards`.
///
/// Portage de `enrich_cards_rows`. Une carte sans correspondance YGOPRODeck est
/// laissée **intacte** — ses colonnes de statistiques restent `None`, ce qui est
/// exactement le critère de comptage journalisé par la V1.0.4.
///
/// Subtilité de fidélité : quand la carte est trouvée mais qu'un champ manque,
/// le Python écrit `""` et non `None` (`ygo.get("frameType", "")`). On reproduit :
/// la carte compte alors comme enrichie.
pub fn enrichir_cartes(cartes: &mut [LigneCarte], index: &HashMap<i64, &CarteYgoprodeck>) -> usize {
    let mut enrichies = 0;

    for ligne in cartes.iter_mut() {
        let Some(pw) = ligne.ygoprodeck_id else {
            continue;
        };
        let Some(ygo) = index.get(&pw) else {
            continue;
        };

        let banlist = ygo.banlist_info.clone().unwrap_or_default();
        ligne.frame_type = Some(ygo.frame_type.clone().unwrap_or_default());
        ligne.atk = ygo.atk.as_ref().and_then(vers_i64);
        ligne.def = ygo.def.as_ref().and_then(vers_i64);
        ligne.level = ygo.level.as_ref().and_then(vers_i64);
        ligne.attribute = Some(ygo.attribute.clone().unwrap_or_default());
        ligne.race = Some(ygo.race.clone().unwrap_or_default());
        ligne.banlist_tcg = Some(banlist.ban_tcg.unwrap_or_default());
        ligne.banlist_ocg = Some(banlist.ban_ocg.unwrap_or_default());
        enrichies += 1;
    }

    enrichies
}

/// Résout les UUID d'illustration et génère une ligne par artwork.
///
/// Portage de `expand_prints_for_multi_art`. Deux corrections en une passe :
///
/// **Résolution.** `contents[].cards[].id` de YGOJSON est un identifiant de
/// tirage, pas d'illustration. On le remplace par le vrai `card_images.uuid`,
/// trouvé via les passwords.
///
/// **Expansion.** Quand YGOPRODeck confirme plusieurs passwords pour le même
/// `(set_code, rareté)` et que ces passwords appartiennent bien à la carte, on
/// génère une ligne par illustration. Exemple canonique : `DUSA-EN072`
/// Rescue Cat, deux artworks, donc deux lignes.
///
/// Les quatre cas, dans l'ordre où le Python les traite :
///
/// | Situation | Résultat |
/// |---|---|
/// | la carte n'a aucun password connu | tirage inchangé |
/// | pas de `set_code` ou pas de rareté | UUID du **premier** password de la carte |
/// | aucun password confirmé pour ce `(code, rareté)` | UUID du **premier** password de la carte |
/// | un seul confirmé | cet UUID |
/// | plusieurs confirmés | **une ligne par UUID** |
///
/// Catalogue vide (YGOPRODeck indisponible) : les tirages sont renvoyés
/// inchangés.
pub fn resoudre_et_etendre_artworks(
    tirages: Vec<TirageBrut>,
    catalogue: &[CarteYgoprodeck],
    images: &[LigneImage],
) -> Vec<TirageBrut> {
    if catalogue.is_empty() {
        return tirages;
    }

    // Index 1 : password → uuid d'illustration. Premier vu, gagnant.
    let mut password_vers_image: HashMap<i64, &str> = HashMap::new();
    for image in images {
        if let Some(pw) = image.ygoprodeck_image_id {
            password_vers_image.entry(pw).or_insert(&image.uuid);
        }
    }

    // Index 2 : uuid de carte → passwords de ses illustrations, dans l'ordre.
    let mut carte_vers_passwords: HashMap<&str, Vec<i64>> = HashMap::new();
    for image in images {
        let Some(pw) = image.ygoprodeck_image_id else {
            continue;
        };
        if image.card_uuid.is_empty() {
            continue;
        }
        let liste = carte_vers_passwords
            .entry(image.card_uuid.as_str())
            .or_default();
        if !liste.contains(&pw) {
            liste.push(pw);
        }
    }

    // Index 3 : (set_code, rareté) → passwords confirmés par YGOPRODeck.
    let mut code_rarete_vers_passwords: HashMap<(&str, &str), Vec<i64>> = HashMap::new();
    for carte in catalogue {
        let Some(pw) = carte.id.as_ref().and_then(vers_i64) else {
            continue;
        };
        for tirage in &carte.card_sets {
            if tirage.set_code.is_empty() {
                continue;
            }
            let liste = code_rarete_vers_passwords
                .entry((tirage.set_code.as_str(), tirage.set_rarity.as_str()))
                .or_default();
            if !liste.contains(&pw) {
                liste.push(pw);
            }
        }
    }

    let mut sortie: Vec<TirageBrut> = Vec::with_capacity(tirages.len());

    for tirage in tirages {
        let passwords_carte = carte_vers_passwords
            .get(tirage.card_uuid.as_str())
            .map(Vec::as_slice)
            .unwrap_or(&[]);

        if passwords_carte.is_empty() {
            sortie.push(tirage);
            continue;
        }

        let premier_uuid = passwords_carte
            .first()
            .and_then(|pw| password_vers_image.get(pw))
            .map(|s| (*s).to_owned());

        let Some(code) = tirage.set_code.as_deref().filter(|c| !c.is_empty()) else {
            sortie.push(TirageBrut {
                card_image_uuid: premier_uuid,
                ..tirage
            });
            continue;
        };
        if tirage.rarity.is_empty() {
            sortie.push(TirageBrut {
                card_image_uuid: premier_uuid,
                ..tirage
            });
            continue;
        }

        let confirmes = code_rarete_vers_passwords
            .get(&(code, tirage.rarity.as_str()))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let passwords_artwork: Vec<i64> = confirmes
            .iter()
            .copied()
            .filter(|pw| passwords_carte.contains(pw))
            .collect();

        match passwords_artwork.len() {
            0 | 1 => {
                let uuid = passwords_artwork
                    .first()
                    .and_then(|pw| password_vers_image.get(pw))
                    .map(|s| (*s).to_owned())
                    .or(premier_uuid);
                sortie.push(TirageBrut {
                    card_image_uuid: uuid,
                    ..tirage
                });
            }
            _ => {
                // Une ligne par illustration. Un password sans illustration
                // connue est sauté — il ne produit pas de ligne orpheline.
                for pw in passwords_artwork {
                    let Some(uuid) = password_vers_image.get(&pw) else {
                        continue;
                    };
                    sortie.push(TirageBrut {
                        card_image_uuid: Some((*uuid).to_owned()),
                        ..tirage.clone()
                    });
                }
            }
        }
    }

    sortie
}

// ─────────────────────────────────────────────────────────────────────────────
// Création de classeur — le repli par set
// ─────────────────────────────────────────────────────────────────────────────

/// Liste complète des sets (`cardsets.php`).
pub const URL_SETS: &str = "https://db.ygoprodeck.com/api/v7/cardsets.php";

/// Cartes d'un set (`cardinfo.php?cardset=…`).
pub const URL_CARTES_DU_SET: &str = "https://db.ygoprodeck.com/api/v7/cardinfo.php";

/// Enveloppe de `cardinfo.php` : les cartes sont sous `data`.
#[derive(Debug, Clone, Deserialize)]
struct ReponseSet<T> {
    #[serde(default = "Vec::new")]
    data: Vec<T>,
}

/// Tous les sets connus de YGOPRODeck.
///
/// Portage de `_fetch_cardsets`. Sert à traduire un préfixe (`RA05`) en nom
/// complet — le seul identifiant que `cardinfo.php` accepte.
///
/// Contrairement au Python, l'erreur **remonte** : c'est l'appelant qui décide
/// qu'une panne réseau n'est pas fatale, et il le fait explicitement.
pub async fn lister_sets<T: serde::de::DeserializeOwned>(client: &ClientHttp) -> Result<Vec<T>> {
    let sets: Vec<T> = client.get_json(URL_SETS).await?;
    Ok(sets)
}

/// Cartes d'un set, dans la langue demandée.
///
/// Portage de `_fetch_set_cards`. `langue` vide demande l'anglais, qui est le
/// défaut de l'API.
///
/// # Une asymétrie du Python, reproduite par l'appelant
///
/// Le Python **avale** toute erreur ici et rend une liste vide. C'est
/// délibéré côté français — un nom manquant n'empêche pas de créer le
/// classeur — mais côté anglais la liste vide devient « aucune carte trouvée »
/// et fait échouer la création. Cette fonction rend donc l'erreur ; c'est
/// `ygo-app::creation` qui l'absorbe pour le français et pas pour l'anglais,
/// là où la distinction se lit.
pub async fn cartes_du_set<T: serde::de::DeserializeOwned>(
    client: &ClientHttp,
    nom_set: &str,
    langue: &str,
) -> Result<Vec<T>> {
    let mut url = format!("{URL_CARTES_DU_SET}?cardset={}", urlencodage(nom_set));
    if !langue.is_empty() {
        url.push_str(&format!("&language={}", urlencodage(langue)));
    }
    let reponse: ReponseSet<T> = client.get_json(&url).await?;
    Ok(reponse.data)
}

/// Encode une valeur de paramètre de requête.
///
/// Les noms de set contiennent des espaces, des deux-points et des apostrophes
/// (« Legendary Collection Kaiba Mega Pack », « Duelist's Advance ») : les
/// passer bruts donnerait une URL invalide.
fn urlencodage(valeur: &str) -> String {
    let mut sortie = String::with_capacity(valeur.len());
    for octet in valeur.bytes() {
        match octet {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                sortie.push(octet as char);
            }
            b' ' => sortie.push_str("%20"),
            _ => sortie.push_str(&format!("%{octet:02X}")),
        }
    }
    sortie
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn image(uuid: &str, carte: &str, pw: Option<i64>) -> LigneImage {
        LigneImage {
            uuid: uuid.to_owned(),
            card_uuid: carte.to_owned(),
            ygoprodeck_image_id: pw,
            art_url: String::new(),
            card_url: String::new(),
        }
    }

    fn tirage(carte: &str, code: Option<&str>, rarete: &str) -> TirageBrut {
        TirageBrut {
            set_uuid: "s1".into(),
            locale_key: "en".into(),
            card_uuid: carte.to_owned(),
            card_image_uuid: Some("uuid-de-tirage-yugipedia".into()),
            set_code: code.map(str::to_owned),
            rarity: rarete.to_owned(),
            edition: "1st".into(),
            qty: 1,
            print_image_url: None,
        }
    }

    fn carte_ygo(pw: i64, tirages: &[(&str, &str)]) -> CarteYgoprodeck {
        CarteYgoprodeck {
            id: Some(serde_json::json!(pw)),
            card_sets: tirages
                .iter()
                .map(|(c, r)| TirageYgoprodeck {
                    set_code: (*c).to_owned(),
                    set_rarity: (*r).to_owned(),
                })
                .collect(),
            ..CarteYgoprodeck::default()
        }
    }

    // ── Enrichissement ──────────────────────────────────────────────────────

    #[test]
    fn l_enrichissement_recopie_les_statistiques() {
        let ygo = CarteYgoprodeck {
            id: Some(serde_json::json!(89631139)),
            frame_type: Some("normal".into()),
            atk: Some(serde_json::json!(3000)),
            def: Some(serde_json::json!(2500)),
            level: Some(serde_json::json!(8)),
            attribute: Some("LIGHT".into()),
            race: Some("Dragon".into()),
            banlist_info: Some(InfoBanlist {
                ban_tcg: Some("Limited".into()),
                ban_ocg: None,
            }),
            ..CarteYgoprodeck::default()
        };
        let catalogue = vec![ygo];
        let index = indexer_par_password(&catalogue);

        let mut cartes = vec![LigneCarte {
            uuid: "u1".into(),
            ygoprodeck_id: Some(89631139),
            ..LigneCarte::default()
        }];
        assert_eq!(enrichir_cartes(&mut cartes, &index), 1);

        assert!(cartes[0].est_enrichie());
        assert_eq!(cartes[0].atk, Some(3000));
        assert_eq!(cartes[0].def, Some(2500));
        assert_eq!(cartes[0].level, Some(8));
        assert_eq!(cartes[0].attribute.as_deref(), Some("LIGHT"));
        assert_eq!(cartes[0].race.as_deref(), Some("Dragon"));
        assert_eq!(cartes[0].banlist_tcg.as_deref(), Some("Limited"));
        assert_eq!(
            cartes[0].banlist_ocg.as_deref(),
            Some(""),
            "champ absent → chaîne vide"
        );
    }

    #[test]
    fn une_carte_sans_correspondance_reste_intacte() {
        let catalogue = vec![carte_ygo(1, &[])];
        let index = indexer_par_password(&catalogue);
        let mut cartes = vec![
            LigneCarte {
                uuid: "u1".into(),
                ygoprodeck_id: Some(999),
                ..LigneCarte::default()
            },
            LigneCarte {
                uuid: "u2".into(),
                ygoprodeck_id: None,
                ..LigneCarte::default()
            },
        ];
        assert_eq!(enrichir_cartes(&mut cartes, &index), 0);
        assert!(!cartes[0].est_enrichie());
        assert!(!cartes[1].est_enrichie());
    }

    #[test]
    fn une_attaque_variable_ne_casse_pas_l_enrichissement() {
        // L'API renvoie "?" pour les monstres à attaque variable.
        let catalogue = vec![CarteYgoprodeck {
            id: Some(serde_json::json!(1)),
            frame_type: Some("effect".into()),
            atk: Some(serde_json::json!("?")),
            ..CarteYgoprodeck::default()
        }];
        let index = indexer_par_password(&catalogue);
        let mut cartes = vec![LigneCarte {
            ygoprodeck_id: Some(1),
            ..LigneCarte::default()
        }];
        enrichir_cartes(&mut cartes, &index);
        assert!(cartes[0].est_enrichie());
        assert_eq!(cartes[0].atk, None);
    }

    // ── Résolution et expansion ─────────────────────────────────────────────

    #[test]
    fn catalogue_vide_laisse_les_tirages_intacts() {
        let t = vec![tirage("c1", Some("RA05-EN134"), "Ultra Rare")];
        let sortie = resoudre_et_etendre_artworks(t.clone(), &[], &[]);
        assert_eq!(sortie, t, "YGOPRODeck indisponible ne doit rien changer");
    }

    #[test]
    fn l_uuid_de_tirage_est_remplace_par_l_uuid_d_illustration() {
        let images = vec![image("img-A", "c1", Some(111))];
        let catalogue = vec![carte_ygo(111, &[("RA05-EN134", "Ultra Rare")])];

        let sortie = resoudre_et_etendre_artworks(
            vec![tirage("c1", Some("RA05-EN134"), "Ultra Rare")],
            &catalogue,
            &images,
        );
        assert_eq!(sortie.len(), 1);
        assert_eq!(
            sortie[0].card_image_uuid.as_deref(),
            Some("img-A"),
            "l'UUID de tirage YGOJSON doit être remplacé"
        );
    }

    #[test]
    fn deux_artworks_confirmes_donnent_deux_lignes() {
        // Cas canonique : DUSA-EN072 Rescue Cat, deux illustrations.
        let images = vec![
            image("img-A", "c1", Some(111)),
            image("img-B", "c1", Some(222)),
        ];
        let catalogue = vec![
            carte_ygo(111, &[("DUSA-EN072", "Ultra Rare")]),
            carte_ygo(222, &[("DUSA-EN072", "Ultra Rare")]),
        ];

        let sortie = resoudre_et_etendre_artworks(
            vec![tirage("c1", Some("DUSA-EN072"), "Ultra Rare")],
            &catalogue,
            &images,
        );
        assert_eq!(sortie.len(), 2, "une ligne par illustration");
        let uuids: Vec<&str> = sortie
            .iter()
            .filter_map(|t| t.card_image_uuid.as_deref())
            .collect();
        assert_eq!(uuids, vec!["img-A", "img-B"]);
        // Le reste du tirage est recopié à l'identique.
        assert_eq!(sortie[0].set_code.as_deref(), Some("DUSA-EN072"));
        assert_eq!(sortie[1].rarity, "Ultra Rare");
    }

    #[test]
    fn une_carte_sans_password_connu_reste_inchangee() {
        let images = vec![image("img-A", "autre-carte", Some(111))];
        let catalogue = vec![carte_ygo(111, &[("X-EN001", "Common")])];
        let entree = tirage("c1", Some("X-EN001"), "Common");

        let sortie = resoudre_et_etendre_artworks(vec![entree.clone()], &catalogue, &images);
        assert_eq!(sortie, vec![entree]);
    }

    #[test]
    fn sans_code_de_set_on_prend_le_premier_password_de_la_carte() {
        let images = vec![
            image("img-A", "c1", Some(111)),
            image("img-B", "c1", Some(222)),
        ];
        let catalogue = vec![carte_ygo(111, &[])];

        let sortie =
            resoudre_et_etendre_artworks(vec![tirage("c1", None, "Common")], &catalogue, &images);
        assert_eq!(sortie.len(), 1);
        assert_eq!(sortie[0].card_image_uuid.as_deref(), Some("img-A"));
    }

    #[test]
    fn sans_confirmation_on_retombe_sur_le_premier_password() {
        let images = vec![image("img-A", "c1", Some(111))];
        // Le catalogue ne connaît pas ce (code, rareté).
        let catalogue = vec![carte_ygo(111, &[("AUTRE-EN001", "Common")])];

        let sortie = resoudre_et_etendre_artworks(
            vec![tirage("c1", Some("X-EN001"), "Common")],
            &catalogue,
            &images,
        );
        assert_eq!(sortie[0].card_image_uuid.as_deref(), Some("img-A"));
    }

    #[test]
    fn un_password_sans_illustration_est_saute_sans_ligne_orpheline() {
        let images = vec![image("img-A", "c1", Some(111))];
        // 222 appartient à la carte selon le catalogue, mais aucune image ne le
        // porte : il ne doit pas produire une ligne à `card_image_uuid` nul.
        let catalogue = vec![
            carte_ygo(111, &[("X-EN001", "Common")]),
            carte_ygo(222, &[("X-EN001", "Common")]),
        ];

        let sortie = resoudre_et_etendre_artworks(
            vec![tirage("c1", Some("X-EN001"), "Common")],
            &catalogue,
            &images,
        );
        // 222 n'est pas dans les passwords de la carte (aucune image), donc un
        // seul confirmé → une seule ligne.
        assert_eq!(sortie.len(), 1);
        assert_eq!(sortie[0].card_image_uuid.as_deref(), Some("img-A"));
    }

    #[test]
    fn l_index_par_password_garde_la_premiere_occurrence() {
        let catalogue = vec![
            CarteYgoprodeck {
                id: Some(serde_json::json!(1)),
                frame_type: Some("a".into()),
                ..Default::default()
            },
            CarteYgoprodeck {
                id: Some(serde_json::json!(1)),
                frame_type: Some("b".into()),
                ..Default::default()
            },
        ];
        let index = indexer_par_password(&catalogue);
        assert_eq!(index.len(), 1);
        assert_eq!(index[&1].frame_type.as_deref(), Some("a"));
    }

    #[test]
    fn le_catalogue_se_deserialise() {
        let json = r#"{"data":[
            {"id":89631139,"frameType":"normal","atk":3000,"def":2500,"level":8,
             "attribute":"LIGHT","race":"Dragon",
             "banlist_info":{"ban_tcg":"Limited"},
             "card_sets":[{"set_code":"LOB-EN001","set_rarity":"Ultra Rare"}]}
        ]}"#;
        let r: ReponseCatalogue = serde_json::from_str(json).unwrap();
        assert_eq!(r.data.len(), 1);
        assert_eq!(r.data[0].card_sets[0].set_code, "LOB-EN001");
        assert_eq!(
            r.data[0]
                .banlist_info
                .as_ref()
                .and_then(|b| b.ban_ocg.clone()),
            None
        );
    }

    #[test]
    fn l_encodage_d_url_couvre_les_noms_de_set_reels() {
        assert_eq!(
            urlencodage("Legendary Collection Kaiba Mega Pack"),
            "Legendary%20Collection%20Kaiba%20Mega%20Pack"
        );
        assert_eq!(urlencodage("Duelist's Advance"), "Duelist%27s%20Advance");
        assert_eq!(
            urlencodage("Rarity Collection II: Quarter Century"),
            "Rarity%20Collection%20II%3A%20Quarter%20Century"
        );
        assert_eq!(urlencodage("RA05"), "RA05", "rien à encoder");
        assert_eq!(
            urlencodage("Pharaoh's Servant"),
            "Pharaoh%27s%20Servant",
            "l'apostrophe doit être encodée, sinon l'URL est invalide"
        );
    }
}

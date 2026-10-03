// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! L'image de booster d'un classeur.
//!
//! Portage de `img_dl/booster_service.py`, **avec une correction**.
//!
//! # Le bug du Python : une table de langues qui ne correspond à rien
//!
//! `get_booster_url` traduit le suffixe régional d'un préfixe OCG en code de
//! langue, par une table écrite en dur :
//!
//! ```text
//! JP/JA → "ja"   KR/KO → "ko"   AE → "ae"   SC/TC → "zh"
//! ```
//!
//! Or `set_locales.language` de la base réelle ne contient **aucune** de ces
//! valeurs, sauf `ae`. Relevé sur la `cardinfo.db` de l'utilisateur :
//!
//! ```text
//! jp 1465   en 1052   de 768   fr 762   it 760   sp 721   kr 616
//! pt 489    sc 160    ae 115   na 77    eu 41    tc 18    fc 4   oc 3
//! ```
//!
//! **Zéro ligne en `ja`, zéro en `ko`, zéro en `zh`.** La recherche d'image de
//! booster ne pouvait donc aboutir pour aucun classeur japonais, coréen,
//! chinois simplifié ou traditionnel — c'est-à-dire pour tous les classeurs
//! OCG. `LOCH-JP` a bien une URL en base ; le Python ne pouvait pas la
//! trouver.
//!
//! La correction tient en un mot : le code de langue est le **suffixe en
//! minuscules**. `LOCH-JP` → `jp`, et les cinq valeurs présentes dans les
//! données tombent juste. Les deux suffixes qui n'existent nulle part (`JA`,
//! `KO`) ne trouvent simplement rien, ce qui est correct.

use std::path::PathBuf;

use rusqlite::Connection;
use ygo_core::config::a_suffixe_ocg;
use ygo_core::paths::Paths;

use crate::error::Result;

/// L'URL de l'image de booster d'un préfixe, lue dans `cardinfo.db`.
///
/// - Préfixe **TCG nu** (`RA02`) : cherché dans les locales `en` et `eu`, en
///   ôtant le suffixe des préfixes stockés — un set TCG est enregistré sous
///   `RA02-EN`.
/// - Préfixe **OCG complet** (`LOCH-JP`) : égalité exacte sur le préfixe, dans
///   la locale du suffixe en minuscules.
///
/// Rend `None` quand la base est absente, la table pas encore créée, ou
/// l'URL vide.
pub fn url(conn: &Connection, prefixe: &str) -> Option<String> {
    let prefixe = prefixe.trim().to_uppercase();
    if prefixe.is_empty() {
        return None;
    }
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'set_locales'",
        [],
        |_| Ok(()),
    )
    .ok()?;

    let url: Option<String> = if a_suffixe_ocg(&prefixe) {
        let langue = prefixe.rsplit('-').next()?.to_lowercase();
        conn.query_row(
            "SELECT booster_image_url FROM set_locales \
             WHERE UPPER(prefix) = ?1 AND language = ?2 \
               AND booster_image_url IS NOT NULL AND booster_image_url != '' \
             LIMIT 1",
            (&prefixe, &langue),
            |l| l.get(0),
        )
        .ok()
    } else {
        conn.query_row(
            "SELECT booster_image_url FROM set_locales \
             WHERE SUBSTR(prefix, 1, INSTR(prefix || '-', '-') - 1) = ?1 \
               AND language IN ('en', 'eu') \
               AND booster_image_url IS NOT NULL AND booster_image_url != '' \
             LIMIT 1",
            [&prefixe],
            |l| l.get(0),
        )
        .ok()
    };
    url.map(|u| u.trim().to_owned()).filter(|u| !u.is_empty())
}

/// L'extension à donner au fichier local, déduite de l'URL.
///
/// Les URL Yugipedia portent parfois une révision — `…%21LOCH-JP-BP.png` —
/// donc l'extension se lit sur le **dernier point du dernier segment**, et
/// seules les extensions d'image connues sont retenues. À défaut, `.png` :
/// c'est ce que le Python fait, et toutes les couvertures présentes chez
/// l'utilisateur sont des PNG.
#[must_use]
pub fn extension(url: &str) -> &'static str {
    let sans_requete = url.split(['?', '#']).next().unwrap_or(url);
    let dernier = sans_requete.rsplit('/').next().unwrap_or("");
    match dernier.rsplit('.').next().map(str::to_lowercase).as_deref() {
        Some("jpg" | "jpeg") => ".jpg",
        Some("webp") => ".webp",
        Some("gif") => ".gif",
        _ => ".png",
    }
}

/// Où la couverture d'un classeur doit atterrir.
#[must_use]
pub fn destination(paths: &Paths, code: &str, url: &str) -> PathBuf {
    paths
        .img_boosters()
        .join(format!("{}{}", code.to_uppercase(), extension(url)))
}

/// Les visuels de set que Yugipedia héberge, par fréquence décroissante.
///
/// Mesuré sur les **5 401 URL de couverture** que `cardinfo.db` porte
/// réellement : `Booster` 2 233, `Promo` 1 170, `Deck` 748, `Logo` 190,
/// `VideoGame` 90, `Box` 59, `Sneak` 58. `Logo` et `VideoGame` sont écartés —
/// ce ne sont pas des visuels de produit.
pub const VISUELS: [&str; 5] = ["Booster", "Deck", "Promo", "Box", "Cover"];

/// Le nom de fichier Yugipedia d'une couverture, s'il en porte la forme.
///
/// La convention, lue sur les mêmes 5 401 URL :
///
/// ```text
/// EP1-BoosterEN.jpg      LOCH-JP-BP.png      KC01-PromoJP.png
/// <préfixe>-<Visuel><LANGUE>.<extension>
/// ```
///
/// Rend le visuel reconnu et sa langue.
#[must_use]
pub fn visuel_de(fichier: &str, prefixe: &str) -> Option<(String, String)> {
    let base = fichier.rsplit_once('.').map_or(fichier, |(b, _)| b);
    let reste = base.strip_prefix(prefixe)?.strip_prefix('-')?;
    for visuel in VISUELS {
        if let Some(langue) = reste.strip_prefix(visuel) {
            // La langue est le suffixe en deux lettres majuscules ; certains
            // fichiers n'en portent pas.
            if langue.is_empty() || (langue.len() == 2 && langue.chars().all(char::is_uppercase)) {
                return Some((visuel.to_owned(), langue.to_owned()));
            }
        }
    }
    None
}

/// Choisit la meilleure couverture parmi des fichiers Yugipedia.
///
/// Fonction **pure**, pour être éprouvée sans réseau. L'ordre de préférence :
/// la langue du classeur d'abord — un `CH02-DeckJP` sur un classeur `EN`
/// serait le bon produit dans la mauvaise édition — puis le rang de
/// [`VISUELS`].
#[must_use]
pub fn choisir<'a>(
    fichiers: &'a [(String, String)],
    prefixe: &str,
    langue: &str,
) -> Option<&'a str> {
    let langue = langue.to_uppercase();
    let mut meilleur: Option<(usize, bool, &str)> = None;
    for (nom, url) in fichiers {
        let Some((visuel, langue_fichier)) = visuel_de(nom, prefixe) else {
            continue;
        };
        let rang = VISUELS
            .iter()
            .position(|v| *v == visuel)
            .unwrap_or(usize::MAX);
        // Une langue absente n'exclut pas : `LOCH-JP-BP` n'en porte aucune.
        let bonne_langue = langue_fichier.is_empty() || langue_fichier == langue;
        if !bonne_langue {
            continue;
        }
        let candidat = (rang, langue_fichier.is_empty(), url.as_str());
        if meilleur.is_none_or(|(r, vide, _)| (rang, langue_fichier.is_empty()) < (r, vide)) {
            meilleur = Some(candidat);
        }
    }
    meilleur.map(|(_, _, url)| url)
}

/// Cherche une couverture sur Yugipedia, quand `cardinfo.db` n'en a pas.
///
/// # Pourquoi ce repli existe
///
/// `booster_image_url` est **vide pour 15 % des sets EN** de la base — mesuré
/// le 2026-09-05 après le signalement de `CH02`, dont les dix locales sont
/// toutes vides chez YGOJSON. Le Python avait ce repli (« Cover LOCR-JP
/// récupérée via Yugipedia (fallback) » dans son journal du 2026-08-25) ; ce
/// portage ne l'avait pas, et une couverture manquante n'avait alors aucune
/// seconde chance.
///
/// Une seule requête `allimages`, celle que la passe artworks fait déjà pour
/// ses propres besoins : on ne devine aucun nom de fichier, on lit ce que
/// Yugipedia déclare et on choisit.
async fn depuis_yugipedia(
    client: &ygo_sources::ClientHttp,
    prefixe: &str,
    langue: &str,
) -> Option<String> {
    let fichiers = ygo_sources::yugipedia::artwork::lister_images(client, prefixe)
        .await
        .ok()?;
    let paires: Vec<(String, String)> = fichiers
        .into_iter()
        .map(|f| (f.nom.replace('_', " "), f.url))
        .collect();
    let choix = choisir(&paires, prefixe, langue).map(ToOwned::to_owned);
    match &choix {
        Some(url) => tracing::info!(prefixe, url = %url, "couverture trouvée via Yugipedia"),
        None => tracing::warn!(prefixe, "aucune couverture chez Yugipedia non plus"),
    }
    choix
}

/// La langue attendue pour la couverture d'un préfixe.
///
/// `LOCH-JP` → `JP` ; un préfixe TCG nu → `EN`.
#[must_use]
pub fn langue_de(prefixe: &str) -> String {
    if a_suffixe_ocg(prefixe) {
        prefixe
            .rsplit_once('-')
            .map_or_else(|| "EN".to_owned(), |(_, s)| s.to_uppercase())
    } else {
        "EN".to_owned()
    }
}

/// Télécharge la couverture d'un classeur si elle manque.
///
/// Rend le chemin écrit, ou `None` s'il n'y avait rien à faire — couverture
/// déjà présente, URL introuvable, téléchargement refusé.
pub async fn telecharger(
    paths: &Paths,
    client: &ygo_sources::ClientHttp,
    code: &str,
) -> Result<Option<PathBuf>> {
    // Une couverture déjà là, sous n'importe quelle extension, suffit.
    if paths.chercher_cover(code).is_some() {
        return Ok(None);
    }
    let Ok(conn) = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db()) else {
        return Ok(None);
    };
    // La base d'abord — c'est gratuit. Yugipedia ensuite, pour les 15 % de
    // sets dont YGOJSON ne donne aucune image.
    let adresse = match url(&conn, code) {
        Some(a) => a,
        None => match depuis_yugipedia(client, code, &langue_de(code)).await {
            Some(a) => a,
            None => return Ok(None),
        },
    };
    let cible = destination(paths, code, &adresse);

    // Le quota est attendu par `get_ok`, avant chaque envoi — pas ici.
    // Une seule requête Yugipedia en vol, corps compris.
    let _jeton = ygo_sources::http::en_serie(&adresse).await;
    let Ok(reponse) = client.get_ok(&adresse).await else {
        return Ok(None);
    };
    let Ok(octets) = reponse.bytes().await else {
        return Ok(None);
    };
    // Même garde que pour les images de cartes : une page d'erreur rendue en
    // 200 OK ne doit pas s'installer comme couverture.
    if (octets.len() as u64) < ygo_images::TAILLE_MINIMALE {
        return Ok(None);
    }
    ygo_images::ecrire_atomique(&cible, &octets)
        .map_err(|e| crate::error::AppError::Creation(e.to_string()))?;
    Ok(Some(cible))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn base() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE set_locales (prefix TEXT, language TEXT, booster_image_url TEXT)",
            (),
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO set_locales VALUES ('RA02-EN', 'en', 'https://x.tld/RA02.png');
             INSERT INTO set_locales VALUES ('LOCH-JP', 'jp', 'https://x.tld/LOCH.png');
             INSERT INTO set_locales VALUES ('VIDE-EN', 'en', '');
             INSERT INTO set_locales VALUES ('NUL-EN', 'en', NULL);
             INSERT INTO set_locales VALUES ('BLANC-EN', 'en', '   ');
             INSERT INTO set_locales VALUES ('KOR-KR', 'kr', 'https://x.tld/KOR.jpg');",
        )
        .unwrap();
        conn
    }

    /// Le cœur de la correction : la langue d'un préfixe OCG est son suffixe
    /// **en minuscules**. La table du Python cherchait `ja` là où la base
    /// écrit `jp` — donc aucune couverture OCG n'était jamais trouvée.
    #[test]
    fn un_prefixe_ocg_cherche_dans_la_langue_de_son_suffixe() {
        let conn = base();
        assert_eq!(
            url(&conn, "LOCH-JP").as_deref(),
            Some("https://x.tld/LOCH.png")
        );
        assert_eq!(
            url(&conn, "KOR-KR").as_deref(),
            Some("https://x.tld/KOR.jpg"),
            "le coréen aussi : la base écrit « kr », pas « ko »"
        );
    }

    #[test]
    fn un_prefixe_tcg_ote_le_suffixe_des_prefixes_stockes() {
        let conn = base();
        assert_eq!(
            url(&conn, "RA02").as_deref(),
            Some("https://x.tld/RA02.png"),
            "le set est stocké sous RA02-EN"
        );
        assert_eq!(
            url(&conn, "ra02").as_deref(),
            Some("https://x.tld/RA02.png")
        );
    }

    #[test]
    fn une_url_vide_ou_absente_ne_rend_rien() {
        let conn = base();
        assert_eq!(url(&conn, "VIDE"), None);
        assert_eq!(url(&conn, "NUL"), None);
        // Le SQL écarte la chaîne vide, mais pas une URL faite d'espaces :
        // c'est le filtre côté Rust qui la retient, et il doit exister.
        assert_eq!(
            url(&conn, "BLANC"),
            None,
            "une URL blanche n'est pas une URL"
        );
        assert_eq!(url(&conn, "INCONNU"), None);
        assert_eq!(url(&conn, ""), None);
        assert_eq!(url(&conn, "   "), None);
    }

    #[test]
    fn une_table_absente_ne_fait_pas_lever() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(url(&conn, "RA02"), None);
    }

    /// L'extension se lit sur le dernier segment, et seules les extensions
    /// d'image connues sont retenues — une URL d'archive Yugipedia porte une
    /// révision et des `%21` qui ne doivent pas devenir une extension.
    #[test]
    fn l_extension_se_deduit_de_l_url_avec_repli_sur_png() {
        assert_eq!(extension("https://x.tld/LOCH.png"), ".png");
        assert_eq!(extension("https://x.tld/LOCH.JPG"), ".jpg");
        assert_eq!(extension("https://x.tld/LOCH.jpeg"), ".jpg");
        assert_eq!(extension("https://x.tld/LOCH.webp"), ".webp");
        assert_eq!(extension("https://x.tld/LOCH.png?v=2"), ".png");
        assert_eq!(
            extension("https://ms.yugipedia.com//archive/7/7d/20251221095727%21LOCH-JP-BP"),
            ".png",
            "pas d'extension reconnaissable : repli"
        );
        assert_eq!(extension(""), ".png");
    }

    #[test]
    fn la_destination_porte_le_code_en_majuscules() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        assert_eq!(
            destination(&paths, "loch-jp", "https://x.tld/a.png"),
            paths.img_boosters().join("LOCH-JP.png")
        );
        assert_eq!(
            destination(&paths, "RA02", "https://x.tld/a.jpg"),
            paths.img_boosters().join("RA02.jpg")
        );
    }

    /// La convention lue sur les 5 401 URL réelles de `cardinfo.db`.
    #[test]
    fn les_noms_de_visuel_se_reconnaissent() {
        assert_eq!(
            visuel_de("EP1-BoosterEN.jpg", "EP1"),
            Some(("Booster".to_owned(), "EN".to_owned()))
        );
        assert_eq!(
            visuel_de("KC01-PromoJP.png", "KC01"),
            Some(("Promo".to_owned(), "JP".to_owned()))
        );
        assert_eq!(
            visuel_de("CH02-DeckEN.png", "CH02"),
            Some(("Deck".to_owned(), "EN".to_owned()))
        );
        // Un artwork de carte n'est pas une couverture.
        assert_eq!(
            visuel_de("AbominationsPrison-RA02-EN-CR-1E.png", "RA02"),
            None
        );
        // Ni un visuel d'un autre set.
        assert_eq!(visuel_de("EP1-BoosterEN.jpg", "CH02"), None);
    }

    /// L'ordre de préférence : la langue du classeur d'abord, le rang du
    /// visuel ensuite.
    #[test]
    fn la_langue_du_classeur_prime_sur_le_rang_du_visuel() {
        let fichiers = vec![
            ("CH02-BoosterJP.png".to_owned(), "url-jp".to_owned()),
            ("CH02-DeckEN.png".to_owned(), "url-en".to_owned()),
        ];
        assert_eq!(
            choisir(&fichiers, "CH02", "EN"),
            Some("url-en"),
            "un Booster dans la mauvaise langue perd contre un Deck dans la bonne"
        );
        assert_eq!(choisir(&fichiers, "CH02", "JP"), Some("url-jp"));
    }

    #[test]
    fn a_langue_egale_le_rang_du_visuel_tranche() {
        let fichiers = vec![
            ("CH02-PromoEN.png".to_owned(), "promo".to_owned()),
            ("CH02-BoosterEN.png".to_owned(), "booster".to_owned()),
        ];
        assert_eq!(choisir(&fichiers, "CH02", "EN"), Some("booster"));
    }

    /// `LOCH-JP-BP` ne porte pas de langue : l'exclure serait perdre la seule
    /// couverture disponible.
    #[test]
    fn un_fichier_sans_langue_reste_recevable() {
        let fichiers = vec![("SDLI-Cover.png".to_owned(), "sans-langue".to_owned())];
        assert_eq!(choisir(&fichiers, "SDLI", "EN"), Some("sans-langue"));
    }

    #[test]
    fn rien_de_reconnaissable_ne_donne_rien() {
        let fichiers = vec![("BlueEyes-SDWD-EN-UR-1E.png".to_owned(), "carte".to_owned())];
        assert_eq!(choisir(&fichiers, "SDWD", "EN"), None);
    }

    /// La langue attendue vient du suffixe OCG, ou vaut EN.
    #[test]
    fn la_langue_se_deduit_du_prefixe() {
        assert_eq!(langue_de("LOCH-JP"), "JP");
        assert_eq!(langue_de("LOCR-JP"), "JP");
        assert_eq!(langue_de("RA05"), "EN");
        assert_eq!(langue_de("CH02"), "EN");
    }
}

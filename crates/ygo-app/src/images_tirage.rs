// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! L'image Yugipedia **du tirage**, posée dans la ligne du classeur.
//!
//! # Pourquoi — 2026-09-30
//!
//! Le réglage « Source des images : Yugipedia » ne choisit pas d'image : il
//! prend l'URL **stockée** dans la ligne ([`ygo_core::image_source::url_image`]).
//! Or les deux chemins de création stockent une URL YGOPRODeck — une par
//! illustration, la même pour toutes les raretés d'une carte. Mesuré sur les
//! 16 classeurs de l'utilisateur : 334 lignes sur 4 328 portaient une URL
//! Yugipedia, toutes posées par la passe artworks. Le réglage n'agissait que
//! sur elles.
//!
//! `cardinfo.db` porte pourtant, dans `set_prints.print_image_url`, l'image
//! Yugipedia **de chaque tirage** — `TheFallenTheVirtuous-CH01-EN-ScR-1E.png`
//! pour la Secret Rare 1re édition, une autre pour l'Ultra Rare. Ce module la
//! retrouve et la pose.
//!
//! # Sans risque pour la source YGOPRODeck
//!
//! Sous YGOPRODeck, l'URL téléchargée est **reconstruite** depuis
//! `card_image_id` ; l'URL stockée n'y sert que si l'identifiant manque. Poser
//! une URL Yugipedia ne change donc rien à qui a choisi YGOPRODeck, et changer
//! de source reste réversible dans les deux sens.
//!
//! # La règle
//!
//! Pour une ligne `(set_code, rareté)`, les tirages de même `set_code` et de
//! même rareté **canonique**. Parmi eux, l'édition de la ligne si elle est
//! renseignée, puis `1st`, `unlimited`, `limited` — la première édition qui
//! porte une image décide :
//!
//! - **une** image : elle est retenue ;
//! - **plusieurs** images distinctes : ambiguïté, on ne choisit **pas**. Vu
//!   sur `MDM`, un set qui mêle plusieurs codes (87 lignes sur 3 477 dans la
//!   simulation) ; jamais sur les 42 classeurs réels ;
//! - aucune : la ligne garde son URL.
//!
//! Ne sont jamais touchées : les lignes que la passe artworks a servies
//! (`card_image_uuid` en `yugipedia:`), qui portent déjà une image Yugipedia
//! choisie avec plus d'information ; et les artworks externes (identifiant
//! négatif), dont l'URL stockée est la seule qui existe.
//!
//! # Mesure préalable
//!
//! Simulée sur 82 classeurs avant la moindre ligne de code : 42 réels (16 de
//! l'installation Rust, 26 figés des V1.0.3 et V1.0.4) et 40 sets tirés au
//! hasard. **70,8 %** des 11 464 lignes réelles trouvent leur image, 100 % sur
//! les decks de structure (`CH01` : 62 images distinctes pour 62 tirages).
//! Le reste manque dans YGOJSON — tirage absent (`Grand Master Rare` de
//! `LOCH-JP`, les deux tiers de `RA05`) ou présent sans image (`LDK2`) — et
//! garde exactement son comportement d'avant.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use ygo_core::rarity::canon::canoniser;
use ygo_core::rarity::Priorites;

use crate::error::Result;

/// Ordre de préférence des éditions quand la ligne n'en dit rien.
pub const EDITIONS_PREFEREES: [&str; 3] = ["1st", "unlimited", "limited"];

/// Ce que la règle décide pour une ligne.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choix {
    /// Une image, et une seule.
    Url(String),
    /// Plusieurs images distinctes pour la même édition : on ne tranche pas.
    Ambigu,
    /// Aucune image dans les tirages trouvés.
    Aucune,
}

/// Choisit l'image parmi les tirages `(édition, url)` d'une ligne.
///
/// Fonction **pure** : la lecture de `cardinfo.db` est ailleurs.
///
/// ```
/// use ygo_app::images_tirage::{choisir, Choix};
/// let t = |e: &str, u: &str| (e.to_owned(), u.to_owned());
/// let tirages = [t("unlimited", "UE.png"), t("1st", "1E.png")];
/// // Sans édition sur la ligne, la 1re édition passe devant.
/// assert_eq!(choisir(&tirages, None), Choix::Url("1E.png".into()));
/// // L'édition de la ligne passe devant tout.
/// assert_eq!(choisir(&tirages, Some("unlimited")), Choix::Url("UE.png".into()));
/// // Deux images pour la même édition : on ne choisit pas.
/// assert_eq!(choisir(&[t("1st", "a.png"), t("1st", "b.png")], None), Choix::Ambigu);
/// // La même image répétée n'est pas une ambiguïté.
/// assert_eq!(choisir(&[t("1st", "a.png"), t("1st", "a.png")], None), Choix::Url("a.png".into()));
/// ```
#[must_use]
pub fn choisir(tirages: &[(String, String)], edition_ligne: Option<&str>) -> Choix {
    let propre = edition_ligne.map(str::trim).filter(|e| !e.is_empty());
    let ordre = propre.into_iter().chain(
        EDITIONS_PREFEREES
            .iter()
            .copied()
            .filter(|e| Some(*e) != propre),
    );
    for edition in ordre {
        let urls: BTreeSet<&str> = tirages
            .iter()
            .filter(|(e, u)| e == edition && !u.is_empty())
            .map(|(_, u)| u.as_str())
            .collect();
        match urls.len() {
            0 => {}
            1 => {
                return urls
                    .into_iter()
                    .next()
                    .map_or(Choix::Aucune, |u| Choix::Url(u.to_owned()))
            }
            _ => return Choix::Ambigu,
        }
    }
    Choix::Aucune
}

/// Les tirages de `cardinfo.db`, par `(set_code, rareté canonique)`.
#[derive(Debug, Default)]
pub struct Index {
    tirages: BTreeMap<(String, String), Vec<(String, String)>>,
    codes_lus: BTreeSet<String>,
}

impl Index {
    /// Lit les tirages des `set_code` donnés — un par un, sur l'index de la
    /// colonne. Un code déjà lu ne l'est pas deux fois.
    ///
    /// # Errors
    ///
    /// Rend une erreur si `set_prints` n'est pas lisible.
    pub fn charger<'a>(
        cardinfo: &Connection,
        codes: impl IntoIterator<Item = &'a str>,
        reference: &Priorites,
    ) -> Result<Self> {
        let mut index = Self::default();
        let mut requete = cardinfo.prepare(
            "SELECT COALESCE(rarity,''), COALESCE(edition,''), COALESCE(print_image_url,'') \
               FROM set_prints WHERE set_code = ?1",
        )?;
        for code in codes {
            if code.is_empty() || !index.codes_lus.insert(code.to_owned()) {
                continue;
            }
            let lignes = requete
                .query_map([code], |l| {
                    Ok((
                        l.get::<_, String>(0)?,
                        l.get::<_, String>(1)?,
                        l.get::<_, String>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (rarete, edition, url) in lignes {
                let Some(canon) = canoniser(&rarete, reference) else {
                    continue;
                };
                index
                    .tirages
                    .entry((code.to_owned(), canon.libelle))
                    .or_default()
                    .push((edition, url));
            }
        }
        Ok(index)
    }

    /// Les tirages d'une ligne, s'il y en a.
    #[must_use]
    pub fn tirages(&self, set_code: &str, rarete_canonique: &str) -> Option<&[(String, String)]> {
        self.tirages
            .get(&(set_code.to_owned(), rarete_canonique.to_owned()))
            .map(Vec::as_slice)
    }
}

/// Ce que l'analyse d'un classeur a trouvé, sans rien écrire.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rapport {
    /// Lignes examinées.
    pub lues: usize,
    /// `(rowid, url)` à écrire.
    pub a_poser: Vec<(i64, String)>,
    /// Lignes qui portent déjà l'image de leur tirage.
    pub deja: usize,
    /// Lignes servies par la passe artworks — laissées telles quelles.
    pub servies_artworks: usize,
    /// Artworks externes — laissés tels quels.
    pub externes: usize,
    /// Aucun tirage de ce `set_code` et de cette rareté dans `cardinfo.db`.
    pub sans_tirage: usize,
    /// Tirage trouvé, mais sans image.
    pub sans_image: usize,
    /// Plusieurs images possibles — on n'a pas choisi.
    pub ambigues: usize,
    /// Rareté qu'aucune table ne reconnaît.
    pub rarete_inconnue: usize,
    /// Lignes sans tirage qui portaient une image d'emprunt, rendues à leur
    /// image YGOPRODeck — comptées aussi dans `a_poser` (cf. `rendre`).
    pub rendues: usize,
}

impl Rapport {
    /// Lignes qui auront, après écriture, l'image de leur tirage.
    #[must_use]
    pub fn couvertes(&self) -> usize {
        self.a_poser.len() - self.rendues + self.deja
    }
}

/// Une ligne du classeur, réduite à ce que la règle lit.
struct Ligne {
    rowid: i64,
    set_code: String,
    rarete: String,
    edition: Option<String>,
    image_id: Option<i64>,
    image_uuid: String,
    url: String,
}

/// Décide, sans rien écrire, quelle ligne reçoit quelle image.
///
/// # Errors
///
/// Rend une erreur si l'une des deux bases n'est pas lisible.
pub fn analyser(
    classeur: &Connection,
    cardinfo: &Connection,
    reference: &Priorites,
) -> Result<Rapport> {
    // Trois des classeurs réels n'ont pas de colonne `edition` : la demander
    // ferait échouer la requête entière.
    let a_edition = classeur
        .prepare("SELECT 1 FROM pragma_table_info('cards') WHERE name = 'edition'")?
        .query_row([], |_| Ok(()))
        .optional()?
        .is_some();
    let colonne_edition = if a_edition { "edition" } else { "NULL" };
    let lignes: Vec<Ligne> = {
        let mut requete = classeur.prepare(&format!(
            "SELECT rowid, COALESCE(set_code,''), COALESCE(rarity,''), {colonne_edition}, \
                    card_image_id, COALESCE(card_image_uuid,''), COALESCE(card_image_url,'') \
               FROM cards"
        ))?;
        let lignes = requete
            .query_map([], |l| {
                Ok(Ligne {
                    rowid: l.get(0)?,
                    set_code: l.get(1)?,
                    rarete: l.get(2)?,
                    edition: l.get(3)?,
                    image_id: l.get(4)?,
                    image_uuid: l.get(5)?,
                    url: l.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        lignes
    };

    let index = Index::charger(
        cardinfo,
        lignes.iter().map(|l| l.set_code.as_str()),
        reference,
    )?;
    let mut rapport = Rapport {
        lues: lignes.len(),
        ..Rapport::default()
    };
    for ligne in &lignes {
        if ligne.image_uuid.starts_with("yugipedia:") {
            rapport.servies_artworks += 1;
            continue;
        }
        if ygo_core::image_source::est_image_externe(ligne.image_id) {
            rapport.externes += 1;
            continue;
        }
        let choix = match canoniser(&ligne.rarete, reference) {
            None => {
                rapport.rarete_inconnue += 1;
                None
            }
            Some(canon) => match index.tirages(&ligne.set_code, &canon.libelle) {
                None => {
                    rapport.sans_tirage += 1;
                    None
                }
                Some(tirages) => match choisir(tirages, ligne.edition.as_deref()) {
                    Choix::Url(url) => Some(url),
                    Choix::Ambigu => {
                        rapport.ambigues += 1;
                        None
                    }
                    Choix::Aucune => {
                        rapport.sans_image += 1;
                        None
                    }
                },
            },
        };
        match choix {
            Some(url) if url == ligne.url => rapport.deja += 1,
            Some(url) => rapport.a_poser.push((ligne.rowid, url)),
            None => {
                if let Some(url) = rendre(ligne) {
                    rapport.rendues += 1;
                    rapport.a_poser.push((ligne.rowid, url));
                }
            }
        }
    }
    Ok(rapport)
}

/// L'URL YGOPRODeck à rendre à une ligne qui porte une image Yugipedia
/// **d'emprunt**.
///
/// La passe artworks crée un tirage manquant en recopiant une ligne sœur,
/// `card_image_url` comprise. Une fois les images de tirage posées, la sœur
/// porte l'image de **sa** rareté : la ligne créée hérite donc d'une image qui
/// n'est pas la sienne. Si elle trouve son propre tirage, elle le reçoit ; si
/// elle n'en trouve pas, elle retrouve l'image YGOPRODeck de son illustration
/// — commune à toutes les raretés, donc juste — plutôt que de garder celle
/// d'une autre rareté.
///
/// Une URL Yugipedia hors de la passe artworks ne peut venir que d'ici : sur
/// les 16 classeurs de l'utilisateur, les 334 URL Yugipedia existantes sont
/// **toutes** marquées `yugipedia:` (mesuré le 2026-09-30).
fn rendre(ligne: &Ligne) -> Option<String> {
    let id = ligne.image_id.filter(|id| *id > 0)?;
    ligne
        .url
        .contains("yugipedia.com")
        .then(|| ygo_core::image_source::url_ygoprodeck(id))
}

/// Écrit les URL décidées par [`analyser`], en une transaction.
///
/// Seule `card_image_url` change : ni la possession, ni la quantité, ni
/// l'état, ni l'édition. La garde sur `card_image_uuid` est rejouée dans le
/// `WHERE`, pour qu'une passe artworks survenue entre l'analyse et l'écriture
/// ne soit pas écrasée.
///
/// # Errors
///
/// Rend une erreur si la transaction échoue — rien n'est alors écrit.
pub fn appliquer(classeur: &mut Connection, rapport: &Rapport) -> Result<usize> {
    let transaction = classeur.transaction()?;
    let mut ecrites = 0;
    {
        let mut maj = transaction.prepare(
            "UPDATE cards SET card_image_url = ?1 WHERE rowid = ?2 \
               AND (card_image_uuid IS NULL OR card_image_uuid NOT LIKE 'yugipedia:%')",
        )?;
        for (rowid, url) in &rapport.a_poser {
            ecrites += maj.execute(rusqlite::params![url, rowid])?;
        }
    }
    transaction.commit()?;
    Ok(ecrites)
}

/// Analyse puis écrit, sur un classeur désigné par son chemin.
///
/// C'est la forme qu'emploie la création. Connexion **simple** au classeur,
/// comme partout sur ce chemin : une connexion WAL y laisserait un `-wal`
/// orphelin, ce que le test de purge des dossiers résiduels a déjà attrapé
/// deux fois.
///
/// # Errors
///
/// Rend une erreur si l'une des deux bases est illisible ou si l'écriture
/// échoue.
pub fn poser(
    chemin_classeur: &Path,
    chemin_cardinfo: &Path,
    reference: &Priorites,
) -> Result<Rapport> {
    let cardinfo = ygo_db::connexion::ouvrir_lecture_seule(chemin_cardinfo)?;
    let mut classeur = Connection::open(chemin_classeur)?;
    let rapport = analyser(&classeur, &cardinfo, reference)?;
    if !rapport.a_poser.is_empty() {
        appliquer(&mut classeur, &rapport)?;
    }
    Ok(rapport)
}

/// Ce que la mise à jour vers Yugipedia ferait — ou a fait — sur toute
/// l'installation.
///
/// C'est ce que montre l'écran des options avant qu'on clique, et ce que la
/// tâche de fond rend après : les mêmes chiffres, calculés par le même code.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Installation {
    /// Classeurs examinés.
    pub classeurs: usize,
    /// Lignes examinées, tous classeurs confondus.
    pub lignes: usize,
    /// Lignes qui recevront (ou ont reçu) l'image de leur tirage.
    pub a_poser: usize,
    /// Lignes qui l'ont déjà.
    pub deja: usize,
    /// Les classeurs qui ont au moins une ligne à poser — eux seuls auront
    /// des images à télécharger.
    pub touches: Vec<String>,
    /// Images à télécharger ensuite : fichiers distincts absents du disque.
    ///
    /// Une estimation honnête, pas une promesse : une image que Yugipedia
    /// n'a pas finira par la source de repli, mais elle aura été demandée.
    pub a_telecharger: usize,
}

/// Analyse — ou met à jour, si `ecrire` — tous les classeurs de
/// l'installation.
///
/// Seule `card_image_url` change, classeur par classeur, chacun dans sa
/// transaction (cf. [`appliquer`]). Un classeur illisible est journalisé et
/// sauté : il ne doit pas priver les autres de leurs images.
///
/// # Errors
///
/// Rend une erreur si `cardinfo.db` est illisible — sans elle, aucun tirage ne
/// peut être retrouvé.
pub fn installation(
    paths: &ygo_core::paths::Paths,
    reference: &Priorites,
    ecrire: bool,
) -> Result<Installation> {
    let cardinfo = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db())?;
    let mut bilan = Installation::default();
    let mut fichiers: BTreeSet<std::path::PathBuf> = BTreeSet::new();

    for code in paths.classeurs_existants() {
        let chemin = paths.classeur_db(&code);
        if !chemin.is_file() {
            continue;
        }
        let mut conn = match Connection::open(&chemin) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(classeur = %code, erreur = %e, "classeur illisible");
                continue;
            }
        };
        let rapport = match analyser(&conn, &cardinfo, reference) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(classeur = %code, erreur = %e, "classeur non analysé");
                continue;
            }
        };
        bilan.classeurs += 1;
        bilan.lignes += rapport.lues;
        bilan.deja += rapport.deja;
        bilan.a_poser += rapport.a_poser.len() - rapport.rendues;

        // Les fichiers à venir : une URL Yugipedia dont le fichier n'est ni
        // dans `img/small` ni dans le dossier du classeur.
        let dossier = paths.img().join(&code);
        for (_, url) in &rapport.a_poser {
            if !url.contains("yugipedia.com") {
                continue;
            }
            if let Some(nom) = ygo_images::plan::nom_de_fichier(url) {
                let small = paths.img_small().join(&nom);
                if !small.exists() && !dossier.join(&nom).exists() {
                    fichiers.insert(small);
                }
            }
        }

        if rapport.a_poser.is_empty() {
            continue;
        }
        bilan.touches.push(code.clone());
        if ecrire {
            if let Err(e) = appliquer(&mut conn, &rapport) {
                tracing::warn!(classeur = %code, erreur = %e, "images de tirage non écrites");
                bilan.touches.pop();
            }
        }
    }
    bilan.a_telecharger = fichiers.len();
    Ok(bilan)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn t(e: &str, u: &str) -> (String, String) {
        (e.to_owned(), u.to_owned())
    }

    fn reference() -> Priorites {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("rarity_config.json");
        std::fs::write(
            &chemin,
            r#"{"Common": 1, "Ultra Rare": 4, "Secret Rare": 5, "Starlight Rare": 21}"#,
        )
        .unwrap();
        Priorites::charger(&chemin)
    }

    fn cardinfo(tirages: &[(&str, &str, &str, &str)]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE set_prints (set_code TEXT, rarity TEXT, edition TEXT, print_image_url TEXT)",
        )
        .unwrap();
        for (code, rarete, edition, url) in tirages {
            conn.execute(
                "INSERT INTO set_prints VALUES (?1, ?2, ?3, ?4)",
                [code, rarete, edition, url],
            )
            .unwrap();
        }
        conn
    }

    /// Le schéma réel d'un classeur, pour que la requête tourne contre lui.
    fn classeur() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
        conn
    }

    fn ligne(conn: &Connection, code: &str, rarete: &str, id: i64, uuid: &str, url: &str) {
        conn.execute(
            "INSERT INTO cards (set_code, rarity, card_image_id, card_image_uuid, card_image_url, \
                                quantite, possessed) VALUES (?1, ?2, ?3, ?4, ?5, 0, 0)",
            rusqlite::params![code, rarete, id, uuid, url],
        )
        .unwrap();
    }

    fn urls(conn: &Connection) -> Vec<String> {
        let mut r = conn
            .prepare("SELECT card_image_url FROM cards ORDER BY rowid")
            .unwrap();
        r.query_map([], |l| l.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    /// Toute l'installation, de l'analyse à l'écriture : l'analyse
    /// n'écrit rien, l'écriture pose, et rejouée elle ne trouve plus rien.
    #[test]
    fn l_installation_entiere_s_analyse_puis_s_ecrit() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = ygo_core::paths::Paths::depuis_racine(tmp.path());
        std::fs::create_dir_all(paths.bdd()).unwrap();
        {
            let info = Connection::open(paths.cardinfo_db()).unwrap();
            info.execute_batch(
                "CREATE TABLE set_prints (set_code TEXT, rarity TEXT, edition TEXT, print_image_url TEXT)",
            )
            .unwrap();
            for (r, u) in [("Ultra Rare", "UR"), ("Secret Rare", "ScR")] {
                info.execute(
                    "INSERT INTO set_prints VALUES ('CH01-EN019', ?1, '1st', ?2)",
                    [r, &format!("https://ms.yugipedia.com//a/b/FV-{u}-1E.png")],
                )
                .unwrap();
            }
        }
        std::fs::create_dir_all(paths.dossier_classeur("CH01")).unwrap();
        {
            let cl = Connection::open(paths.classeur_db("CH01")).unwrap();
            cl.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
            let ygo = "https://images.ygoprodeck.com/images/cards/30271097.jpg";
            for r in ["Ultra Rare", "Secret Rare"] {
                ligne(&cl, "CH01-EN019", r, 30_271_097, "", ygo);
            }
        }

        let vue = installation(&paths, &reference(), false).unwrap();
        assert_eq!((vue.classeurs, vue.lignes, vue.a_poser), (1, 2, 2));
        assert_eq!(vue.touches, ["CH01"]);
        assert_eq!(vue.a_telecharger, 2, "deux fichiers distincts, absents");
        let cl = Connection::open(paths.classeur_db("CH01")).unwrap();
        assert!(
            urls(&cl).iter().all(|u| u.contains("ygoprodeck")),
            "rien d'écrit"
        );

        let faite = installation(&paths, &reference(), true).unwrap();
        assert_eq!(faite.a_poser, 2);
        assert!(urls(&cl).iter().all(|u| u.contains("yugipedia")));

        let encore = installation(&paths, &reference(), false).unwrap();
        assert_eq!((encore.a_poser, encore.deja), (0, 2));
        assert!(encore.touches.is_empty());
    }

    #[test]
    fn une_edition_inconnue_sur_la_ligne_retombe_sur_l_ordre_par_defaut() {
        let tirages = [t("unlimited", "UE.png")];
        assert_eq!(choisir(&tirages, Some("rien")), Choix::Url("UE.png".into()));
        assert_eq!(choisir(&tirages, Some("  ")), Choix::Url("UE.png".into()));
    }

    #[test]
    fn une_edition_sans_image_laisse_passer_la_suivante() {
        let tirages = [t("1st", ""), t("unlimited", "UE.png")];
        assert_eq!(choisir(&tirages, None), Choix::Url("UE.png".into()));
        assert_eq!(choisir(&[t("1st", "")], None), Choix::Aucune);
        assert_eq!(choisir(&[], None), Choix::Aucune);
    }

    /// Le cas `CH01-EN019` : même illustration YGOPRODeck pour les trois
    /// raretés, trois images Yugipedia distinctes.
    #[test]
    fn chaque_rarete_recoit_l_image_de_son_tirage() {
        let info = cardinfo(&[
            ("CH01-EN019", "Ultra Rare", "1st", "FV-UR-1E.png"),
            ("CH01-EN019", "Secret Rare", "1st", "FV-ScR-1E.png"),
            ("CH01-EN019", "Starlight Rare", "1st", "FV-StR-1E.png"),
        ]);
        let mut cl = classeur();
        let ygo = "https://images.ygoprodeck.com/images/cards/30271097.jpg";
        for r in ["Ultra Rare", "Secret Rare", "Starlight Rare"] {
            ligne(&cl, "CH01-EN019", r, 30_271_097, "", ygo);
        }
        let rapport = analyser(&cl, &info, &reference()).unwrap();
        assert_eq!(rapport.a_poser.len(), 3);
        assert_eq!(appliquer(&mut cl, &rapport).unwrap(), 3);
        assert_eq!(
            urls(&cl),
            ["FV-UR-1E.png", "FV-ScR-1E.png", "FV-StR-1E.png"]
        );

        // Rejoué, rien de plus.
        let encore = analyser(&cl, &info, &reference()).unwrap();
        assert!(encore.a_poser.is_empty());
        assert_eq!(encore.deja, 3);
    }

    #[test]
    fn les_lignes_servies_par_la_passe_artworks_et_les_externes_restent() {
        let info = cardinfo(&[("RA02-EN001", "Ultra Rare", "1st", "TIRAGE.png")]);
        let cl = classeur();
        ligne(
            &cl,
            "RA02-EN001",
            "Ultra Rare",
            1,
            "yugipedia:42",
            "ARTWORK-CHOISI.png",
        );
        ligne(&cl, "RA02-EN001", "Ultra Rare", -7, "", "EXTERNE.png");
        let rapport = analyser(&cl, &info, &reference()).unwrap();
        assert!(rapport.a_poser.is_empty());
        assert_eq!(rapport.servies_artworks, 1);
        assert_eq!(rapport.externes, 1);
    }

    #[test]
    fn ce_qui_manque_garde_son_url_et_est_compte() {
        let info = cardinfo(&[
            ("X-001", "Ultra Rare", "1st", ""),
            ("X-002", "Ultra Rare", "1st", "a.png"),
            ("X-002", "Ultra Rare", "1st", "b.png"),
        ]);
        let mut cl = classeur();
        ligne(&cl, "X-001", "Ultra Rare", 1, "", "avant-1.jpg");
        ligne(&cl, "X-002", "Ultra Rare", 2, "", "avant-2.jpg");
        ligne(&cl, "X-003", "Ultra Rare", 3, "", "avant-3.jpg");
        ligne(&cl, "X-004", "New", 4, "", "avant-4.jpg");
        let rapport = analyser(&cl, &info, &reference()).unwrap();
        assert_eq!(rapport.sans_image, 1);
        assert_eq!(rapport.ambigues, 1);
        assert_eq!(rapport.sans_tirage, 1);
        assert_eq!(rapport.rarete_inconnue, 1);
        assert_eq!(appliquer(&mut cl, &rapport).unwrap(), 0);
        assert_eq!(
            urls(&cl),
            ["avant-1.jpg", "avant-2.jpg", "avant-3.jpg", "avant-4.jpg"]
        );
    }

    /// Une ligne créée par la passe artworks hérite de l'URL de sa sœur. Sans
    /// tirage à elle, elle retrouve l'image YGOPRODeck de son illustration.
    #[test]
    fn une_image_d_emprunt_sans_tirage_est_rendue_a_ygoprodeck() {
        let info = cardinfo(&[("A-001", "Secret Rare", "1st", "A-ScR.png")]);
        let mut cl = classeur();
        let emprunt = "https://ms.yugipedia.com//x/A-ScR.png";
        ligne(&cl, "A-001", "Ultra Rare", 42, "", emprunt);
        let rapport = analyser(&cl, &info, &reference()).unwrap();
        assert_eq!(rapport.sans_tirage, 1);
        assert_eq!(rapport.rendues, 1);
        assert_eq!(rapport.couvertes(), 0);
        appliquer(&mut cl, &rapport).unwrap();
        assert_eq!(
            urls(&cl),
            ["https://images.ygoprodeck.com/images/cards/42.jpg"]
        );
        // Une URL YGOPRODeck sans tirage, elle, ne bouge pas.
        let encore = analyser(&cl, &info, &reference()).unwrap();
        assert!(encore.a_poser.is_empty());
    }

    /// Un libellé abrégé trouve le tirage écrit en toutes lettres.
    #[test]
    fn la_rarete_est_comparee_sous_sa_forme_canonique() {
        let info = cardinfo(&[("RA02-EN001", "Secret Rare", "1st", "ScR.png")]);
        let cl = classeur();
        ligne(&cl, "RA02-EN001", "ScR", 1, "", "avant.jpg");
        let rapport = analyser(&cl, &info, &reference()).unwrap();
        assert_eq!(rapport.a_poser, vec![(1, "ScR.png".to_owned())]);
    }

    /// Un classeur ancien, sans colonne `edition`, se lit quand même.
    #[test]
    fn un_classeur_sans_colonne_edition_se_lit() {
        let info = cardinfo(&[("A-001", "Common", "1st", "c.png")]);
        let cl = Connection::open_in_memory().unwrap();
        cl.execute_batch(
            "CREATE TABLE cards (set_code TEXT, rarity TEXT, card_image_id INTEGER, \
                                 card_image_uuid TEXT, card_image_url TEXT);
             INSERT INTO cards VALUES ('A-001', 'Common', 5, '', 'avant.jpg');",
        )
        .unwrap();
        let rapport = analyser(&cl, &info, &reference()).unwrap();
        assert_eq!(rapport.a_poser, vec![(1, "c.png".to_owned())]);
    }

    /// L'édition choisie par l'utilisateur l'emporte sur la 1re édition.
    #[test]
    fn l_edition_de_la_ligne_decide() {
        let info = cardinfo(&[
            ("A-001", "Common", "1st", "1E.png"),
            ("A-001", "Common", "unlimited", "UE.png"),
        ]);
        let cl = classeur();
        ligne(&cl, "A-001", "Common", 5, "", "avant.jpg");
        cl.execute("UPDATE cards SET edition = 'unlimited'", [])
            .unwrap();
        let rapport = analyser(&cl, &info, &reference()).unwrap();
        assert_eq!(rapport.a_poser, vec![(1, "UE.png".to_owned())]);
    }
}

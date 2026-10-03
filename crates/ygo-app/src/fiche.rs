// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! La fiche d'une carte : ce que le classeur ne montre pas.
//!
//! # Le texte était là depuis le début
//!
//! `cardinfo.db` porte une table `card_texts(card_uuid, language, name,
//! effect)` remplie pour **dix langues**. Sur l'installation réelle :
//! 14 607 effets en anglais (toutes les cartes), 13 777 en français,
//! 14 205 en japonais. Ni le Python ni le portage ne l'avaient jamais
//! affichée — le `dialog_carte.py` du Python montre le nom, le code, la
//! rareté et les statistiques, et une vignette de 128 points de large où le
//! texte imprimé est illisible.
//!
//! C'est la **divergence assumée** de ce module : la fiche montre en plus
//! l'illustration à sa définition et le texte de la base, dans la langue
//! choisie. Le déclencheur, lui, ne diverge pas — le Python ouvrait déjà son
//! dialogue au clic sur la carte.
//!
//! # Retrouver la carte : trois voies, dans cet ordre
//!
//! Une ligne de classeur ne porte pas toujours son `card_uuid`. Les classeurs
//! créés par les premières versions ont la colonne vide. Mesure sur les neuf
//! classeurs réels, 1 946 lignes :
//!
//! | voie | lignes |
//! |---|---|
//! | `card_uuid` déjà rempli | 885 |
//! | résolu par `card_image_id` | 927 |
//! | résolu par `set_code` | 134 |
//! | **irrésolu** | **0** |
//!
//! Les trois sont donc nécessaires, et ensemble elles suffisent. C'est
//! pourquoi [`resoudre`] est une cascade et non une jointure unique, et
//! pourquoi l'oracle prend ses cartes dans chacune des trois.

use rusqlite::{Connection, OptionalExtension};

use crate::error::Result;
use ygo_core::config::SourceImage;
use ygo_core::paths::Paths;

/// Par quelle voie le `card_uuid` d'une ligne a été retrouvé.
///
/// Exposé, et pas seulement interne : quand une fiche sort vide, la première
/// question est « par où a-t-on cherché ».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// La ligne de classeur portait son `card_uuid`.
    Uuid,
    /// Retrouvé par `card_image_id` dans `card_images`.
    ParImage,
    /// Retrouvé par `set_code` dans `set_prints`.
    ParCode,
    /// Aucune des trois n'a abouti — la fiche n'aura pas de texte.
    Aucune,
}

impl Resolution {
    /// Le mot que l'oracle emploie.
    #[must_use]
    pub fn mot(self) -> &'static str {
        match self {
            Self::Uuid => "uuid",
            Self::ParImage => "image",
            Self::ParCode => "code",
            Self::Aucune => "aucune",
        }
    }
}

/// Le texte d'une carte dans une langue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texte {
    /// Code de langue tel que la base le stocke (`fr`, `en`, `ja`, `zh-CN`…).
    pub langue: String,
    /// Nom de la carte dans cette langue.
    pub nom: Option<String>,
    /// Texte d'effet.
    pub effet: Option<String>,
}

impl Texte {
    /// Un texte sans effet n'apprend rien : la fiche ne l'offre pas.
    #[must_use]
    pub fn utilisable(&self) -> bool {
        self.effet.as_ref().is_some_and(|e| !e.trim().is_empty())
    }
}

/// Tout ce qu'on sait d'une carte du classeur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fiche {
    /// La ligne, dans son classeur.
    pub rowid: i64,
    /// Nom anglais, tel que le classeur le stocke.
    pub nom: String,
    /// Nom français, vide s'il n'a pas été résolu.
    pub nom_fr: String,
    /// Code de set complet.
    pub set_code: String,
    /// Libellé de rareté.
    pub rarete: String,
    /// Nom du set.
    pub set_name: String,
    /// Type de carte (« Effect Monster », « Spell Card »…).
    pub card_type: String,
    /// Attaque, absente pour une magie ou un piège.
    pub atk: Option<i64>,
    /// Défense.
    pub def: Option<i64>,
    /// Niveau ou rang.
    pub level: Option<i64>,
    /// Attribut (`LIGHT`, `DARK`…).
    pub attribute: String,
    /// Type de monstre.
    pub race: String,
    /// Quantité possédée.
    pub quantite: i64,
    /// État, si renseigné.
    pub qualite: Option<String>,
    /// Édition, si renseignée.
    pub edition: Option<String>,
    /// Illustration étendue.
    pub extended_art: bool,
    /// Identifiant d'illustration.
    pub card_image_id: i64,
    /// Nom du fichier d'illustration, déduit de l'URL retenue.
    ///
    /// Calculé comme dans la liste du classeur, avec la même
    /// [`SourceImage`] : la fiche doit montrer l'image que la case montrait,
    /// pas une autre.
    pub fichier_image: Option<String>,
    /// Par où le texte a été retrouvé.
    pub resolution: Resolution,
    /// Les textes disponibles, une entrée par langue.
    pub textes: Vec<Texte>,
}

/// L'ordre dans lequel les langues sont proposées.
///
/// Celles qui suivent la préférence sont rangées dans un ordre **fixe** et non
/// alphabétique : `zh-CN` avant `de` par ordre alphabétique n'a de sens pour
/// personne. Une langue que ce tableau ignore vient à la fin, dans l'ordre où
/// la base l'a rendue — la base peut en gagner une sans que le code bouge.
pub const ORDRE_LANGUES: [&str; 10] = [
    "fr", "en", "ja", "de", "es", "it", "pt", "ko", "zh-CN", "zh-TW",
];

/// Le rang d'une langue dans [`ORDRE_LANGUES`] ; les inconnues à la fin.
#[must_use]
pub fn rang_langue(langue: &str) -> usize {
    ORDRE_LANGUES
        .iter()
        .position(|l| *l == langue)
        .unwrap_or(ORDRE_LANGUES.len())
}

impl Fiche {
    /// Les langues offertes, la préférée d'abord.
    ///
    /// Seuls les textes **utilisables** comptent : une langue dont l'effet est
    /// vide ferait un onglet qui n'affiche rien.
    #[must_use]
    pub fn langues(&self, preferee: &str) -> Vec<String> {
        let mut offertes: Vec<&Texte> = self.textes.iter().filter(|t| t.utilisable()).collect();
        offertes.sort_by_key(|t| (t.langue != preferee, rang_langue(&t.langue)));
        offertes.iter().map(|t| t.langue.clone()).collect()
    }

    /// Le texte à montrer, la préférence d'abord puis le premier disponible.
    ///
    /// Le repli n'est pas théorique : `LOCR-JP001` n'a que `en`, `ja` et
    /// `zh-CN`. Sur un classeur japonais réglé en français, sans repli la
    /// fiche serait vide alors que la base a le texte.
    #[must_use]
    pub fn texte(&self, preferee: &str) -> Option<&Texte> {
        let langues = self.langues(preferee);
        let premiere = langues.first()?;
        self.textes.iter().find(|t| &t.langue == premiere)
    }

    /// Le texte d'une langue nommée, si elle est utilisable.
    #[must_use]
    pub fn texte_en(&self, langue: &str) -> Option<&Texte> {
        self.textes
            .iter()
            .find(|t| t.langue == langue && t.utilisable())
    }

    /// Le nom à afficher : le français s'il existe et qu'on le demande.
    ///
    /// Même règle que la liste du classeur (`COALESCE(NULLIF(name_fr,''),
    /// name)`), pour que la fiche et la case ne se contredisent pas.
    #[must_use]
    pub fn nom_affiche(&self, francais: bool) -> &str {
        if francais && !self.nom_fr.trim().is_empty() {
            &self.nom_fr
        } else {
            &self.nom
        }
    }

    /// La ligne de statistiques, comme le Python la composait.
    ///
    /// `dialog_carte.py` joint par deux espaces le type puis `ATK/x` puis
    /// `DEF/x`, en sautant ce qui manque. Le niveau et l'attribut, eux, sont
    /// **en plus** : le classeur les stocke et personne ne les montrait.
    ///
    /// ```
    /// use ygo_app::fiche::Fiche;
    /// # fn f(atk: Option<i64>, def: Option<i64>, level: Option<i64>) -> Fiche {
    /// #     Fiche { rowid: 1, nom: String::new(), nom_fr: String::new(),
    /// #         set_code: String::new(), rarete: String::new(), set_name: String::new(),
    /// #         card_type: "Effect Monster".into(), atk, def, level,
    /// #         attribute: "LIGHT".into(), race: "Dragon".into(), quantite: 0,
    /// #         qualite: None, edition: None, extended_art: false, card_image_id: 0,
    /// #         fichier_image: None,
    /// #         resolution: ygo_app::fiche::Resolution::Uuid, textes: Vec::new() }
    /// # }
    /// assert_eq!(
    ///     f(Some(3000), Some(2500), Some(8)).statistiques(),
    ///     "Effect Monster  LIGHT/Dragon  Niv. 8  ATK/3000  DEF/2500"
    /// );
    /// // Une magie n'a ni attaque, ni niveau, ni attribut : rien n'est inventé.
    /// let mut magie = f(None, None, None);
    /// magie.card_type = "Spell Card".into();
    /// magie.attribute = String::new();
    /// magie.race = "Quick-Play".into();
    /// assert_eq!(magie.statistiques(), "Spell Card  Quick-Play");
    /// ```
    #[must_use]
    pub fn statistiques(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.card_type.trim().is_empty() {
            parts.push(self.card_type.clone());
        }
        match (self.attribute.trim(), self.race.trim()) {
            ("", "") => {}
            ("", r) => parts.push(r.to_owned()),
            (a, "") => parts.push(a.to_owned()),
            (a, r) => parts.push(format!("{a}/{r}")),
        }
        if let Some(n) = self.level {
            parts.push(format!("Niv. {n}"));
        }
        if let Some(a) = self.atk {
            parts.push(format!("ATK/{a}"));
        }
        if let Some(d) = self.def {
            parts.push(format!("DEF/{d}"));
        }
        parts.join("  ")
    }

    /// Ce que l'exemplaire possédé a de particulier, ou rien.
    ///
    /// ```
    /// use ygo_app::fiche::possession;
    /// assert_eq!(possession(0, None, None), "non possédée");
    /// assert_eq!(possession(2, None, None), "×2");
    /// assert_eq!(possession(1, Some("NM"), Some("1st")), "×1 · NM (Near Mint) · 1st");
    /// // Un état que la table ne connaît pas passe tel quel.
    /// assert_eq!(possession(1, Some("Neuve"), None), "×1 · Neuve");
    /// // Un champ vide n'est pas un champ renseigné.
    /// assert_eq!(possession(1, Some(""), None), "×1");
    /// ```
    #[must_use]
    pub fn possession(&self) -> String {
        possession(
            self.quantite,
            self.qualite.as_deref(),
            self.edition.as_deref(),
        )
    }
}

/// La ligne « possession » d'une fiche — voir [`Fiche::possession`].
#[must_use]
pub fn possession(quantite: i64, qualite: Option<&str>, edition: Option<&str>) -> String {
    if quantite <= 0 {
        return "non possédée".to_owned();
    }
    let mut parts = vec![format!("×{quantite}")];
    if let Some(q) = qualite.map(str::trim).filter(|q| !q.is_empty()) {
        parts.push(ygo_core::rarity::scanflip::libelle_qualite(q));
    }
    if let Some(e) = edition.map(str::trim).filter(|e| !e.is_empty()) {
        parts.push(e.to_owned());
    }
    parts.join(" · ")
}

/// Le `card_uuid` d'une ligne de classeur, et par quelle voie.
///
/// L'ordre n'est pas indifférent : le `card_uuid` de la ligne est le seul qui
/// distingue deux cartes partageant un `set_code`, et `card_image_id` est plus
/// précis qu'un `set_code` puisqu'il désigne une illustration.
///
/// # Deux requêtes, et pas un index en mémoire
///
/// La première version chargeait `card_images` et `set_prints` dans deux
/// tables de hachage, au motif qu'« ouvrir la base à chaque carte serait payer
/// cher un geste fréquent ». Le motif n'avait jamais été mesuré. Il l'a été
/// après que l'application a visiblement gelé au clic :
///
/// | | coût |
/// |---|---|
/// | construire les deux index (326 450 tirages, 260 000 entrées retenues) | **480 ms** |
/// | interroger `card_images` par son index | 0,05 ms |
/// | interroger `set_prints` par son index | 0,11 ms |
///
/// `idx_card_images_ygo_id` et `idx_set_prints_set_code` existent depuis
/// toujours dans `cardinfo.db` : les requêtes sont des recherches d'index, pas
/// des balayages. J'avais donc construit un quart de million d'entrées à
/// chaque clic pour éviter deux dixièmes de milliseconde.
///
/// # Errors
///
/// Si l'une des deux tables ne se lit pas.
/// La requête qui retrouve une carte par son illustration.
///
/// Exposée pour que le test du plan interroge **celle-ci** et non une copie :
/// un test qui recopie la requête qu'il vérifie ne vérifie que lui-même.
pub const SQL_PAR_IMAGE: &str = "SELECT card_uuid FROM card_images \
     WHERE ygoprodeck_image_id = ? AND card_uuid IS NOT NULL LIMIT 1";

/// La requête qui la retrouve par son code de set.
///
/// `ORDER BY rowid` : le **premier** tirage gagne. Un `set_code` peut porter
/// plusieurs illustrations ; elles désignent la même carte, mais l'ordre doit
/// être stable d'une ouverture à l'autre.
///
/// La mutation qui retire cette clause survit, et c'est correct : avec
/// `idx_set_prints_set_code`, SQLite rend déjà les lignes égales dans l'ordre
/// des `rowid`. La clause ne change rien **tant que ce plan est choisi** —
/// elle est là pour que la garantie ne dépende pas du planificateur.
pub const SQL_PAR_CODE: &str = "SELECT card_uuid FROM set_prints \
     WHERE set_code = ? AND card_uuid IS NOT NULL ORDER BY rowid LIMIT 1";

pub fn resoudre(
    conn: &Connection,
    card_uuid: &str,
    card_image_id: Option<i64>,
    set_code: &str,
) -> Result<(Option<String>, Resolution)> {
    if !card_uuid.trim().is_empty() {
        return Ok((Some(card_uuid.to_owned()), Resolution::Uuid));
    }
    if let Some(id) = card_image_id {
        let mut q = conn.prepare_cached(SQL_PAR_IMAGE)?;
        let trouve: Option<String> = q.query_row([id], |l| l.get(0)).optional()?;
        if let Some(u) = trouve {
            return Ok((Some(u), Resolution::ParImage));
        }
    }
    let mut q = conn.prepare_cached(SQL_PAR_CODE)?;
    let trouve: Option<String> = q.query_row([set_code], |l| l.get(0)).optional()?;
    match trouve {
        Some(u) => Ok((Some(u), Resolution::ParCode)),
        None => Ok((None, Resolution::Aucune)),
    }
}

/// Les textes d'une carte, toutes langues, triés par langue.
///
/// # Errors
///
/// Si la table `card_texts` ne se lit pas.
pub fn textes(conn: &Connection, card_uuid: &str) -> Result<Vec<Texte>> {
    let mut q = conn.prepare(
        "SELECT language, name, effect FROM card_texts WHERE card_uuid = ? ORDER BY language",
    )?;
    let lignes = q.query_map([card_uuid], |l| {
        Ok(Texte {
            langue: l.get(0)?,
            nom: l.get(1)?,
            effet: l.get(2)?,
        })
    })?;
    Ok(lignes.collect::<rusqlite::Result<_>>()?)
}

/// La fiche d'une ligne de classeur, ou `None` si la ligne n'existe pas.
///
/// # Errors
///
/// Si le classeur ou `cardinfo.db` ne se lisent pas.
pub fn lire(
    paths: &Paths,
    classeur: &str,
    rowid: i64,
    source: SourceImage,
) -> Result<Option<Fiche>> {
    let chemin = paths.classeur_db(classeur);
    let conn = ygo_db::connexion::ouvrir_lecture_seule(&chemin)?;
    let ligne = lire_ligne(&conn, rowid, source)?;
    let Some((mut fiche, card_uuid)) = ligne else {
        return Ok(None);
    };

    // La base des cartes peut manquer — une installation neuve, un classeur
    // isolé. La fiche existe quand même, sans texte : mieux vaut le nom, la
    // rareté et l'illustration que rien du tout.
    let cardinfo = paths.cardinfo_db();
    if cardinfo.exists() {
        let info = ygo_db::connexion::ouvrir_lecture_seule(&cardinfo)?;
        let (uuid, voie) = resoudre(
            &info,
            &card_uuid,
            Some(fiche.card_image_id).filter(|id| *id != 0),
            &fiche.set_code,
        )?;
        fiche.resolution = voie;
        if let Some(u) = uuid {
            fiche.textes = textes(&info, &u)?;
        }
    }
    Ok(Some(fiche))
}

/// La ligne du classeur, et son `card_uuid` brut.
fn lire_ligne(
    conn: &Connection,
    rowid: i64,
    source: SourceImage,
) -> Result<Option<(Fiche, String)>> {
    let colonnes = ygo_db::migrations::colonnes(conn, "cards")?;
    let a = |nom: &str| colonnes.iter().any(|c| c == nom);
    // Les classeurs anciens n'ont ni `qualite`, ni `edition`, ni
    // `extended_art` : demander une colonne absente ferait échouer toute la
    // requête, donc on la remplace par une constante.
    let expr_qualite = if a("qualite") { "qualite" } else { "NULL" };
    let expr_edition = if a("edition") { "edition" } else { "NULL" };
    let expr_ext = if a("extended_art") {
        "COALESCE(extended_art, 0)"
    } else {
        "0"
    };
    let expr_uuid = if a("card_uuid") { "card_uuid" } else { "''" };
    let expr_url = if a("card_image_url") {
        "card_image_url"
    } else {
        "NULL"
    };

    let sql = format!(
        "SELECT COALESCE(name, ''), COALESCE(name_fr, ''), COALESCE(set_code, ''), \
                COALESCE(rarity, ''), COALESCE(set_name, ''), COALESCE(card_type, ''), \
                atk, def_val, level, COALESCE(attribute, ''), COALESCE(race, ''), \
                COALESCE(quantite, 0), {expr_qualite}, {expr_edition}, {expr_ext}, \
                COALESCE(card_image_id, 0), COALESCE({expr_uuid}, ''), {expr_url} \
         FROM cards WHERE rowid = ?"
    );
    let mut q = conn.prepare(&sql)?;
    let mut lignes = q.query([rowid])?;
    let Some(l) = lignes.next()? else {
        return Ok(None);
    };
    let card_image_id: i64 = l.get(15)?;
    let url: Option<String> = l.get(17)?;
    let fiche = Fiche {
        rowid,
        nom: l.get(0)?,
        nom_fr: l.get(1)?,
        set_code: l.get(2)?,
        rarete: l.get(3)?,
        set_name: l.get(4)?,
        card_type: l.get(5)?,
        atk: l.get(6)?,
        def: l.get(7)?,
        level: l.get(8)?,
        attribute: l.get(9)?,
        race: l.get(10)?,
        quantite: l.get::<_, i64>(11)?.max(0),
        qualite: l.get(12)?,
        edition: l.get(13)?,
        extended_art: l.get::<_, i64>(14)? != 0,
        card_image_id,
        fichier_image: ygo_core::image_source::url_image(
            source,
            url.as_deref(),
            Some(card_image_id).filter(|id| *id != 0),
        )
        .as_deref()
        .and_then(crate::classeur::fichier_depuis_url),
        resolution: Resolution::Aucune,
        textes: Vec::new(),
    };
    let card_uuid: String = l.get(16)?;
    Ok(Some((fiche, card_uuid)))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;

    fn texte(langue: &str, effet: Option<&str>) -> Texte {
        Texte {
            langue: langue.to_owned(),
            nom: Some(format!("nom {langue}")),
            effet: effet.map(str::to_owned),
        }
    }

    fn fiche(textes: Vec<Texte>) -> Fiche {
        Fiche {
            rowid: 1,
            nom: "Blue-Eyes".into(),
            nom_fr: String::new(),
            set_code: "LOCR-JP001".into(),
            rarete: "Ultra Rare".into(),
            set_name: String::new(),
            card_type: "Effect Monster".into(),
            atk: Some(3000),
            def: Some(2500),
            level: Some(8),
            attribute: "LIGHT".into(),
            race: "Dragon".into(),
            quantite: 1,
            qualite: None,
            edition: None,
            extended_art: false,
            card_image_id: 42,
            fichier_image: None,
            resolution: Resolution::Uuid,
            textes,
        }
    }

    /// Une langue sans effet n'est pas offerte.
    ///
    /// Sans quoi la fiche proposerait un onglet « pt » qui, une fois choisi,
    /// n'affiche rien — et l'utilisateur croit à une panne.
    #[test]
    fn une_langue_sans_effet_n_est_pas_offerte() {
        let f = fiche(vec![
            texte("en", Some("An effect.")),
            texte("pt", None),
            texte("it", Some("   ")),
        ]);
        assert_eq!(f.langues("fr"), vec!["en"]);
        assert!(f.texte_en("pt").is_none());
        assert!(f.texte_en("it").is_none(), "blanc n'est pas du texte");
    }

    /// La préférée passe devant, les autres suivent l'ordre du référentiel.
    #[test]
    fn la_langue_preferee_passe_devant() {
        let f = fiche(vec![
            texte("zh-CN", Some("x")),
            texte("en", Some("x")),
            texte("ja", Some("x")),
            texte("fr", Some("x")),
        ]);
        assert_eq!(f.langues("fr"), vec!["fr", "en", "ja", "zh-CN"]);
        assert_eq!(
            f.langues("ja"),
            vec!["ja", "fr", "en", "zh-CN"],
            "le japonais devant, le reste dans l'ordre du référentiel"
        );
        // Une préférée absente ne perturbe pas le classement.
        assert_eq!(f.langues("de"), vec!["fr", "en", "ja", "zh-CN"]);
    }

    /// Une langue hors référentiel va à la fin, sans faire échouer le tri.
    #[test]
    fn une_langue_inconnue_va_a_la_fin() {
        let f = fiche(vec![texte("xx", Some("x")), texte("en", Some("x"))]);
        assert_eq!(f.langues("fr"), vec!["en", "xx"]);
        assert!(rang_langue("xx") >= ORDRE_LANGUES.len());
    }

    /// Sans français, la fiche montre autre chose plutôt que rien.
    ///
    /// Cas réel et non théorique : `LOCR-JP001` n'a que `en`, `ja` et `zh-CN`.
    #[test]
    fn sans_la_langue_preferee_on_replie_au_lieu_de_ne_rien_montrer() {
        let f = fiche(vec![
            texte("en", Some("An effect.")),
            texte("ja", Some("効果")),
        ]);
        let t = f.texte("fr").expect("un texte, même pas en français");
        assert_eq!(t.langue, "en");
        assert_eq!(t.effet.as_deref(), Some("An effect."));

        // Aucun texte du tout : rien à montrer, et on le dit par `None`.
        assert!(fiche(Vec::new()).texte("fr").is_none());
    }

    /// Le nom suit la même règle que la case du classeur.
    #[test]
    fn le_nom_francais_ne_sert_que_s_il_existe() {
        let mut f = fiche(Vec::new());
        assert_eq!(f.nom_affiche(true), "Blue-Eyes", "pas de nom FR en base");
        f.nom_fr = "Dragon Blanc aux Yeux Bleus".into();
        assert_eq!(f.nom_affiche(true), "Dragon Blanc aux Yeux Bleus");
        assert_eq!(f.nom_affiche(false), "Blue-Eyes", "anglais demandé");
        f.nom_fr = "   ".into();
        assert_eq!(
            f.nom_affiche(true),
            "Blue-Eyes",
            "un blanc n'est pas un nom"
        );
    }

    /// La ligne de statistiques saute ce qui manque, sans laisser de trou.
    #[test]
    fn les_statistiques_sautent_ce_qui_manque() {
        let mut f = fiche(Vec::new());
        assert_eq!(
            f.statistiques(),
            "Effect Monster  LIGHT/Dragon  Niv. 8  ATK/3000  DEF/2500"
        );
        f.atk = None;
        f.def = None;
        f.level = None;
        f.attribute = String::new();
        f.card_type = "Spell Card".into();
        f.race = "Quick-Play".into();
        assert_eq!(f.statistiques(), "Spell Card  Quick-Play");
        // Une carte dont on ne sait rien ne rend pas une ligne de séparateurs.
        f.card_type = String::new();
        f.race = String::new();
        assert_eq!(f.statistiques(), "");
    }

    /// Zéro attaque n'est pas une attaque absente.
    ///
    /// `Deep-Eyes White Dragon` a ATK/0. Confondre `Some(0)` et `None` la
    /// priverait de sa statistique la plus notable.
    #[test]
    fn une_attaque_nulle_s_affiche_quand_meme() {
        let mut f = fiche(Vec::new());
        f.atk = Some(0);
        f.def = Some(0);
        assert!(f.statistiques().contains("ATK/0"), "{}", f.statistiques());
        assert!(f.statistiques().contains("DEF/0"));
    }

    /// La possession se lit d'un coup d'œil, et ne prétend rien de faux.
    #[test]
    fn la_ligne_de_possession_ne_montre_que_ce_qui_est_renseigne() {
        assert_eq!(possession(0, Some("NM"), Some("1st")), "non possédée");
        assert_eq!(possession(-3, None, None), "non possédée", "jamais négatif");
        assert_eq!(possession(1, None, None), "×1");
        assert_eq!(possession(3, Some("NM"), None), "×3 · NM (Near Mint)");
        assert_eq!(possession(1, None, Some("Unlimited")), "×1 · Unlimited");
        assert_eq!(
            possession(1, Some(" "), Some("")),
            "×1",
            "les blancs sautent"
        );
    }

    /// La cascade essaie les trois voies, dans l'ordre du plus précis.
    #[test]
    fn la_resolution_essaie_les_trois_voies_dans_l_ordre() {
        let conn = ygo_db::connexion::en_memoire().unwrap();
        conn.execute_batch(
            "CREATE TABLE card_images (ygoprodeck_image_id INTEGER, card_uuid TEXT);
             CREATE TABLE set_prints (set_code TEXT, card_uuid TEXT);
             INSERT INTO card_images VALUES (42, 'par-image');
             INSERT INTO set_prints  VALUES ('AAA-EN001', 'par-code');
             INSERT INTO set_prints  VALUES ('AAA-EN001', 'second-tirage');",
        )
        .unwrap();
        let r = |u: &str, i: Option<i64>, c: &str| resoudre(&conn, u, i, c).unwrap();

        // Le uuid de la ligne prime sur tout le reste.
        assert_eq!(
            r("le-sien", Some(42), "AAA-EN001"),
            (Some("le-sien".into()), Resolution::Uuid)
        );
        // Puis l'illustration, plus précise que le code de set.
        assert_eq!(
            r("", Some(42), "AAA-EN001"),
            (Some("par-image".into()), Resolution::ParImage)
        );
        // Puis le code de set.
        assert_eq!(
            r("", Some(99), "AAA-EN001"),
            (Some("par-code".into()), Resolution::ParCode),
            "le premier tirage gagne, pas le dernier"
        );
        // Et sinon, rien — dit franchement.
        assert_eq!(r("", None, "ZZZ"), (None, Resolution::Aucune));
        assert_eq!(
            r("   ", Some(99), "ZZZ"),
            (None, Resolution::Aucune),
            "un uuid blanc n'est pas un uuid"
        );
    }

    /// Les index de production, tels que `cardinfo.db` les porte.
    const INDEX_PRODUCTION: &str = "\
        CREATE INDEX idx_card_images_ygo_id ON card_images(ygoprodeck_image_id);\
        CREATE INDEX idx_set_prints_set_code ON set_prints(set_code);";

    /// La résolution passe par un index, jamais par un balayage.
    ///
    /// # La régression que ce test empêche
    ///
    /// La première version chargeait `card_images` et `set_prints` dans deux
    /// tables de hachage à chaque ouverture de fiche : **480 ms** pour
    /// 326 450 tirages, et l'application gelait visiblement au clic. Elle a
    /// été remplacée par deux requêtes à 0,05 et 0,11 ms — mais seulement
    /// parce que SQLite les résout par index.
    ///
    /// Un test de durée serait capricieux selon la machine ; le plan, lui,
    /// est exact. Si la requête cessait d'être une égalité sur la colonne
    /// indexée — un `LIKE`, une fonction autour du champ, une colonne
    /// renommée — SQLite retomberait sur un `SCAN` et le gel reviendrait,
    /// sans qu'aucun autre test ne s'en aperçoive.
    #[test]
    fn la_resolution_passe_par_un_index() {
        let conn = ygo_db::connexion::en_memoire().unwrap();
        conn.execute_batch(
            "CREATE TABLE card_images (ygoprodeck_image_id INTEGER, card_uuid TEXT);
             CREATE TABLE set_prints (set_code TEXT, card_uuid TEXT);",
        )
        .unwrap();
        conn.execute_batch(INDEX_PRODUCTION).unwrap();

        for (sql, parametre) in [(SQL_PAR_IMAGE, "42"), (SQL_PAR_CODE, "AAA-EN001")] {
            let mut q = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let plan: Vec<String> = q
                .query_map([parametre], |l| l.get::<_, String>(3))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            let plan = plan.join(" | ");
            assert!(
                plan.contains("USING INDEX"),
                "cette requête ne passe pas par un index : {plan}"
            );
            assert!(
                !plan.contains("SCAN"),
                "cette requête balaie la table : {plan}"
            );
        }
    }

    /// Un classeur ancien n'a ni qualité, ni édition, ni `card_uuid`.
    ///
    /// Trois des neuf classeurs réels sont dans ce cas. Demander une colonne
    /// absente ferait échouer la requête entière, donc la fiche entière.
    #[test]
    fn un_classeur_ancien_se_lit_sans_ses_colonnes_manquantes() {
        let conn = ygo_db::connexion::en_memoire().unwrap();
        conn.execute_batch(
            "CREATE TABLE cards (name TEXT, name_fr TEXT, set_code TEXT, rarity TEXT,
                 set_name TEXT, card_type TEXT, atk INTEGER, def_val INTEGER,
                 level INTEGER, attribute TEXT, race TEXT, quantite INTEGER,
                 card_image_id INTEGER);
             INSERT INTO cards VALUES ('Obelisk', '', 'EGO1-EN001', 'Ultra Rare',
                 'Egyptian God Deck', 'Effect Monster', 4000, 4000, 10, 'DIVINE',
                 'Divine-Beast', 1, 10000000);",
        )
        .unwrap();
        let (f, uuid) = lire_ligne(&conn, 1, SourceImage::default())
            .unwrap()
            .expect("la ligne existe");
        assert_eq!(f.nom, "Obelisk");
        assert_eq!(f.card_image_id, 10_000_000);
        assert_eq!(f.qualite, None, "colonne absente, pas d'invention");
        assert_eq!(f.edition, None);
        assert!(!f.extended_art);
        assert_eq!(uuid, "", "colonne absente");
        // Sans URL stockée, le nom de fichier se déduit de l'identifiant
        // d'illustration — exactement comme dans la liste du classeur, sans
        // quoi la fiche montrerait une autre image que la case.
        assert_eq!(f.fichier_image.as_deref(), Some("10000000.jpg"));
        // Une ligne qui n'existe pas se dit par `None`, pas par une erreur.
        assert!(lire_ligne(&conn, 99, SourceImage::default())
            .unwrap()
            .is_none());
    }
}

// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les numéros que la Set list Yugipedia connaît et que le classeur ignore.
//!
//! # Pourquoi — 2026-10-10
//!
//! `LOCH-JP013` manquait au classeur, et à `cardinfo.db` avant lui. La carte,
//! elle, y est — *Odd-Eyes Pendulum Dragon of the Four Heavenly Dragons*,
//! statistiques, noms, images —, mais **sans aucun tirage**. La Set Card List
//! de Yugipedia l'écrit « *Odd-Eyes Pendulum Dragon, Four Heavenly Dragons* »,
//! un ancien nom devenu redirection, et YGOJSON, faute de rapprocher les deux,
//! a perdu le tirage. Vérifié dans son archive : le set `LOCH` y va de 001 à
//! 080 sans 013, en japonais comme en chinois.
//!
//! La passe artworks ne pouvait pas le rattraper : elle ajoute des raretés à
//! un numéro **existant**, en recopiant une ligne sœur, et ne crée jamais un
//! numéro absent — elle n'aurait ni statistiques ni nom à écrire.
//!
//! Ce module, si : la carte est dans la base, il suffit de la retrouver.
//!
//! # La règle
//!
//! Pour chaque numéro de la Set list absent du classeur :
//!
//! 1. la carte est cherchée par son **nom anglais exact** dans `cardinfo.db` ;
//! 2. à défaut, Yugipedia donne la fiche de ce nom — redirections suivies —
//!    et son **password** ; la carte est cherchée sous le titre de la fiche,
//!    puis par password, qui ne change jamais — une requête pour cinquante
//!    noms. C'est le password qui retrouve `LOCH-JP013` : la fiche s'appelle
//!    aujourd'hui comme la Set list, la base garde l'autre nom ;
//! 3. une carte et une seule : une ligne par rareté listée, Overframe compris,
//!    avec les statistiques, les noms (le français seulement s'il existe — on
//!    n'invente jamais) et l'illustration de la base ;
//! 4. sinon, **rien n'est créé** : le numéro est rendu comme introuvable.
//!
//! Les lignes naissent non possédées. La passe artworks qui suit leur pose
//! ensuite les images Yugipedia de leurs tirages, comme aux autres.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};
use ygo_core::paths::Paths;
use ygo_core::rarity::Priorites;
use ygo_sources::yugipedia::{EntreeSetList, Fiche};

use crate::creation::{cle_tri, LigneClasseur};
use crate::error::Result;

/// Un numéro absent du classeur, tel que la Set list le décrit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Absent {
    /// Numéro de collection, `LOCH-JP013`.
    pub numero: String,
    /// Le nom écrit dans la Set list — pas forcément celui de la fiche.
    pub nom: String,
    /// `(rareté, extended_art)`, dans l'ordre de la page, sans doublon.
    pub tirages: Vec<(String, i64)>,
}

/// Un numéro ajouté au classeur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ajout {
    /// Numéro de collection.
    pub numero: String,
    /// Nom anglais officiel, tel que la base le connaît.
    pub nom: String,
    /// Le nom écrit dans la Set list.
    pub nom_set_list: String,
    /// Comment la carte a été retrouvée.
    pub via: Rapprochement,
    /// Lignes créées — une par rareté.
    pub lignes: usize,
}

/// Comment la carte d'un numéro absent a été retrouvée dans la base.
///
/// Du plus direct au plus indirect : c'est ce qu'il faut relire en premier
/// si un ajout paraît douteux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rapprochement {
    /// Le nom de la Set list est celui de la base.
    Nom,
    /// Le titre de la fiche Yugipedia — redirection suivie — est celui de la
    /// base.
    Fiche,
    /// Seul le password de la fiche correspond.
    Password,
}

impl Rapprochement {
    /// Le libellé court.
    #[must_use]
    pub fn libelle(self) -> &'static str {
        match self {
            Self::Nom => "nom",
            Self::Fiche => "fiche",
            Self::Password => "password",
        }
    }
}

/// Ce que la complétion a trouvé — et, si elle écrit, fait.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bilan {
    /// Numéros de la Set list absents du classeur.
    pub absents: usize,
    /// Ceux dont la carte a été retrouvée.
    pub ajouts: Vec<Ajout>,
    /// `(numéro, nom de la Set list)` dont la carte reste introuvable.
    pub introuvables: Vec<(String, String)>,
    /// Lignes en cadre normal qui ont reçu l'image Yugipedia de leur tirage
    /// (numéros ajoutés, maintenant ou lors d'un passage précédent).
    pub images: usize,
}

impl Bilan {
    /// Lignes créées (ou à créer), tous numéros confondus.
    #[must_use]
    pub fn lignes(&self) -> usize {
        self.ajouts.iter().map(|a| a.lignes).sum()
    }
}

/// Ce que la complétion d'un classeur a produit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    /// La Set list a été lue.
    Fait(Bilan),
    /// Le classeur est vide, ou sans nom de set.
    ClasseurVide,
    /// Aucune page « Set Card Lists » ne correspond au set.
    PageIntrouvable,
}

/// Le préfixe de numéro d'un classeur : set et région, `LOCH-JP`, `LDK2-EN`.
///
/// La lettre de sous-jeu (`LDK2-ENJ01`) n'en fait pas partie : les quatre
/// sous-decks appartiennent au même classeur.
///
/// ```
/// use ygo_app::numeros_absents::prefixe_numero;
/// assert_eq!(prefixe_numero("LOCH-JP001"), "LOCH-JP");
/// assert_eq!(prefixe_numero("ldk2-enj01"), "LDK2-EN");
/// assert_eq!(prefixe_numero("SDWD-EN013"), "SDWD-EN");
/// ```
#[must_use]
pub fn prefixe_numero(set_code: &str) -> String {
    let code = set_code.trim().to_uppercase();
    match code.split_once('-') {
        Some((set, reste)) => {
            let region: String = reste
                .chars()
                .take_while(char::is_ascii_alphabetic)
                .take(2)
                .collect();
            format!("{set}-{region}")
        }
        None => code,
    }
}

/// Les numéros de la Set list absents du classeur — fonction **pure**.
///
/// Seuls comptent les numéros du préfixe du classeur : une page qui mêlerait
/// plusieurs régions ne doit pas faire entrer des codes étrangers.
#[must_use]
pub fn absents(
    entrees: &[EntreeSetList],
    presents: &HashSet<String>,
    prefixe: &str,
) -> Vec<Absent> {
    let mut par_numero: BTreeMap<String, Absent> = BTreeMap::new();
    for e in entrees {
        let numero = e.numero.trim().to_uppercase();
        if numero.is_empty() || !numero.starts_with(prefixe) || presents.contains(&numero) {
            continue;
        }
        let absent = par_numero.entry(numero.clone()).or_insert_with(|| Absent {
            numero,
            nom: e.nom.trim().to_owned(),
            tirages: Vec::new(),
        });
        let ext = i64::from(e.extended_art);
        for rarete in &e.raretes {
            let rarete = rarete.trim();
            if rarete.is_empty() {
                continue;
            }
            let tirage = (rarete.to_owned(), ext);
            if !absent.tirages.contains(&tirage) {
                absent.tirages.push(tirage);
            }
        }
    }
    par_numero
        .into_values()
        .filter(|a| !a.tirages.is_empty() && !a.nom.is_empty())
        .collect()
}

/// La carte de ce nom anglais, si la base en a **une et une seule**.
fn carte_par_nom(cardinfo: &Connection, nom: &str) -> Result<Option<String>> {
    let mut requete = cardinfo.prepare(
        "SELECT DISTINCT card_uuid FROM card_texts \
         WHERE language = 'en' AND name = ?1 COLLATE NOCASE LIMIT 2",
    )?;
    let uuids: Vec<String> = requete
        .query_map([nom.trim()], |l| l.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(match uuids.as_slice() {
        [un] => Some(un.clone()),
        _ => None,
    })
}

/// La carte de ce password — l'identifiant YGOPRODeck —, si la base en a
/// **une et une seule**.
///
/// C'est le dernier recours, et le plus sûr : le nom d'une carte change
/// (traduction officielle, renommage de fiche), son password jamais.
fn carte_par_password(cardinfo: &Connection, password: i64) -> Result<Option<String>> {
    let mut requete =
        cardinfo.prepare("SELECT DISTINCT uuid FROM cards WHERE ygoprodeck_id = ?1 LIMIT 2")?;
    let uuids: Vec<String> = requete
        .query_map([password], |l| l.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(match uuids.as_slice() {
        [un] => Some(un.clone()),
        _ => None,
    })
}

/// Les lignes d'un numéro absent, pour la carte retrouvée.
fn lignes_de(
    cardinfo: &Connection,
    card_uuid: &str,
    absent: &Absent,
    set_name: &str,
    raretes: &Priorites,
) -> Result<Option<(String, Vec<LigneClasseur>)>> {
    let carte = cardinfo
        .query_row(
            "SELECT c.card_type, c.atk, c.def, c.level, c.attribute, c.race, \
                    (SELECT name FROM card_texts WHERE card_uuid = c.uuid AND language = 'en'), \
                    (SELECT name FROM card_texts WHERE card_uuid = c.uuid AND language = 'fr') \
               FROM cards c WHERE c.uuid = ?1",
            [card_uuid],
            |l| {
                Ok((
                    l.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    l.get::<_, Option<i64>>(1)?,
                    l.get::<_, Option<i64>>(2)?,
                    l.get::<_, Option<i64>>(3)?,
                    l.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    l.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    l.get::<_, Option<String>>(6)?.unwrap_or_default(),
                    l.get::<_, Option<String>>(7)?.unwrap_or_default(),
                ))
            },
        )
        .optional()?;
    let Some((card_type, atk, def_val, level, attribute, race, nom_en, nom_fr)) = carte else {
        return Ok(None);
    };
    // L'illustration YGOPRODeck d'abord : elle a un identifiant, donc une
    // place dans `img/small`, et une image que tout le reste sait servir.
    let image = cardinfo
        .query_row(
            "SELECT uuid, ygoprodeck_image_id, COALESCE(card_url,''), COALESCE(art_url,'') \
               FROM card_images WHERE card_uuid = ?1 \
              ORDER BY (ygoprodeck_image_id IS NULL), ygoprodeck_image_id LIMIT 1",
            [card_uuid],
            |l| {
                Ok((
                    l.get::<_, String>(0)?,
                    l.get::<_, Option<i64>>(1)?,
                    l.get::<_, String>(2)?,
                    l.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    let (image_uuid, image_id, image_url, image_small) = image.unwrap_or_default();

    let lignes = absent
        .tirages
        .iter()
        .map(|(rarete, ext)| LigneClasseur {
            card_uuid: card_uuid.to_owned(),
            card_image_uuid: image_uuid.clone(),
            card_image_id: image_id,
            name: nom_en.clone(),
            name_fr: nom_fr.clone(),
            set_code: absent.numero.clone(),
            // La forme des Options, comme à la création ; un libellé inconnu
            // reste tel quel — jamais rapproché au plus proche.
            rarity: ygo_core::rarity::canon::canoniser(rarete, raretes)
                .map_or_else(|| rarete.clone(), |c| c.libelle),
            rarity_code: String::new(),
            set_name: set_name.to_owned(),
            card_image_url: image_url.clone(),
            card_image_small: image_small.clone(),
            sort_order: 0,
            card_type: card_type.clone(),
            atk,
            def_val,
            level,
            attribute: Some(attribute.clone()),
            race: race.clone(),
            extended_art: *ext,
        })
        .collect();
    Ok(Some((nom_en, lignes)))
}

/// La table `cards` a-t-elle cette colonne ?
fn colonne_existe(conn: &Connection, colonne: &str) -> Result<bool> {
    Ok(conn
        .prepare("SELECT 1 FROM pragma_table_info('cards') WHERE name = ?1")?
        .query_row([colonne], |_| Ok(()))
        .optional()?
        .is_some())
}

/// Écrit les lignes et renumérote le classeur, en une transaction.
///
/// # Errors
///
/// Rend une erreur si la transaction échoue — rien n'est alors écrit.
pub fn ecrire_lignes(conn: &mut Connection, lignes: &[LigneClasseur]) -> Result<usize> {
    let avec_ext = colonne_existe(conn, "extended_art")?;
    let tx = conn.transaction()?;
    {
        let (colonne, valeur) = if avec_ext {
            (", extended_art", ", ?19")
        } else {
            ("", "")
        };
        let mut stmt = tx.prepare(&format!(
            "INSERT INTO cards
                (card_uuid, card_image_uuid, card_image_id, set_code, rarity,
                 rarity_code, set_name, name, name_fr, card_image_url,
                 card_image_small, sort_order, card_type, atk, def_val, level,
                 attribute, race, possessed, quantite{colonne})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18, 0, 0{valeur})"
        ))?;
        // Numéro par numéro : chacun se fait sa place avant d'être écrit.
        let mut par_numero: BTreeMap<&str, Vec<&LigneClasseur>> = BTreeMap::new();
        for l in lignes {
            par_numero.entry(l.set_code.as_str()).or_default().push(l);
        }
        for (numero, groupe) in par_numero {
            let rang = faire_place(&tx, numero)?;
            for l in groupe {
                let mut valeurs: Vec<&dyn rusqlite::ToSql> = vec![
                    &l.card_uuid,
                    &l.card_image_uuid,
                    &l.card_image_id,
                    &l.set_code,
                    &l.rarity,
                    &l.rarity_code,
                    &l.set_name,
                    &l.name,
                    &l.name_fr,
                    &l.card_image_url,
                    &l.card_image_small,
                    &rang,
                    &l.card_type,
                    &l.atk,
                    &l.def_val,
                    &l.level,
                    &l.attribute,
                    &l.race,
                ];
                if avec_ext {
                    valeurs.push(&l.extended_art);
                }
                stmt.execute(valeurs.as_slice())?;
            }
        }
    }
    tx.commit()?;
    Ok(lignes.len())
}

/// Libère un rang pour un numéro, entre ses voisins, et le rend.
///
/// Le rang est celui du **premier numéro qui le suit** ; tout ce qui est à ce
/// rang ou après recule d'un cran. Les lignes déjà là gardent ainsi leur
/// ordre exact — ex æquo compris, que la passe artworks laisse derrière elle :
/// renuméroter tout le classeur aurait déplacé des cartes sous les yeux de
/// l'utilisateur. Sans numéro suivant, le rang est le dernier plus un.
fn faire_place(conn: &Connection, numero: &str) -> Result<i64> {
    let lignes: Vec<(String, i64)> = conn
        .prepare("SELECT COALESCE(set_code,''), COALESCE(sort_order,0) FROM cards")?
        .query_map([], |l| Ok((l.get(0)?, l.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let cle = cle_tri(numero);
    let suivant = lignes
        .iter()
        .filter(|(code, _)| cle_tri(code) > cle)
        .map(|(_, rang)| *rang)
        .min();
    match suivant {
        Some(rang) => {
            conn.execute(
                "UPDATE cards SET sort_order = sort_order + 1 WHERE sort_order >= ?1",
                [rang],
            )?;
            Ok(rang)
        }
        None => Ok(lignes.iter().map(|(_, r)| *r).max().map_or(0, |m| m + 1)),
    }
}

/// Les noms de la Set list que la base ne connaît pas tels quels — ceux qu'il
/// faudra demander à Yugipedia.
///
/// # Errors
///
/// Rend une erreur si `cardinfo.db` est illisible.
pub fn noms_a_resoudre(cardinfo: &Connection, manquants: &[Absent]) -> Result<Vec<String>> {
    let mut noms: Vec<String> = Vec::new();
    for a in manquants {
        if carte_par_nom(cardinfo, &a.nom)?.is_none() && !noms.contains(&a.nom) {
            noms.push(a.nom.clone());
        }
    }
    Ok(noms)
}

/// Décide, numéro par numéro, ce qui sera créé — sans réseau ni écriture.
///
/// `fiches` associe un nom de Set list à sa fiche Yugipedia — titre et
/// password (cf. [`ygo_sources::yugipedia::fiches`]). Un nom trouvé
/// tel quel dans la base n'a pas besoin d'y figurer.
///
/// # Errors
///
/// Rend une erreur si `cardinfo.db` est illisible.
pub fn decider(
    cardinfo: &Connection,
    manquants: &[Absent],
    fiches: &HashMap<String, Fiche>,
    set_name: &str,
    raretes: &Priorites,
) -> Result<(Bilan, Vec<LigneClasseur>)> {
    let mut bilan = Bilan {
        absents: manquants.len(),
        ..Bilan::default()
    };
    let mut lignes_a_ecrire = Vec::new();
    for a in manquants {
        let trouvee = match carte_par_nom(cardinfo, &a.nom)? {
            Some(u) => Some((u, Rapprochement::Nom)),
            None => match fiches.get(&a.nom) {
                Some(fiche) => match carte_par_nom(cardinfo, &fiche.titre)? {
                    Some(u) => Some((u, Rapprochement::Fiche)),
                    None => match fiche.password {
                        Some(p) => {
                            carte_par_password(cardinfo, p)?.map(|u| (u, Rapprochement::Password))
                        }
                        None => None,
                    },
                },
                None => None,
            },
        };
        let construit = match trouvee {
            Some((u, via)) => lignes_de(cardinfo, &u, a, set_name, raretes)?
                .map(|(nom, lignes)| (nom, lignes, via)),
            None => None,
        };
        match construit {
            Some((nom, lignes, via)) => {
                bilan.ajouts.push(Ajout {
                    numero: a.numero.clone(),
                    nom,
                    nom_set_list: a.nom.clone(),
                    via,
                    lignes: lignes.len(),
                });
                lignes_a_ecrire.extend(lignes);
            }
            None => bilan.introuvables.push((a.numero.clone(), a.nom.clone())),
        }
    }
    Ok((bilan, lignes_a_ecrire))
}

/// Les numéros du classeur, en majuscules.
fn numeros_presents(conn: &Connection) -> Result<HashSet<String>> {
    let numeros: Vec<String> = conn
        .prepare("SELECT DISTINCT UPPER(TRIM(set_code)) FROM cards WHERE set_code IS NOT NULL")?
        .query_map([], |l| l.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(numeros.into_iter().collect())
}

/// Cherche — et, si `ecrire`, ajoute — les numéros absents d'un classeur.
///
/// Réseau : la page et la Set list (en cache trente jours, la passe artworks
/// les a souvent lues juste avant), plus une requête pour cinquante noms à
/// résoudre, seulement si un nom n'est pas trouvé tel quel dans la base.
///
/// # Errors
///
/// Rend une erreur si une base est illisible, si l'écriture échoue, ou si
/// Yugipedia ne répond pas.
pub async fn completer(
    paths: &Paths,
    client: &ygo_sources::ClientHttp,
    code: &str,
    raretes: &Priorites,
    ecrire: bool,
) -> Result<Issue> {
    use ygo_sources::yugipedia::{self, artwork, structure};

    // Lecture du classeur, connexion refermée avant le réseau.
    let (inventaire, set_name, presents) = {
        let conn = Connection::open(paths.classeur_db(code))?;
        let (inventaire, set_name) = crate::artworks::lire_inventaire(&conn)?;
        (inventaire, set_name, numeros_presents(&conn)?)
    };
    let (Some(premiere), false) = (inventaire.first(), set_name.is_empty()) else {
        return Ok(Issue::ClasseurVide);
    };
    let code_reference = premiere.set_code.clone();
    let langue = artwork::langue_set_code(&code_reference);

    let titres = yugipedia::resoudre_pages(client, &set_name, None).await?;
    let Some(titre) = structure::choisir_page(&titres, &langue) else {
        return Ok(Issue::PageIntrouvable);
    };
    let (entrees, _) = yugipedia::lire_set_list(client, titre).await?;

    let manquants = absents(&entrees, &presents, &prefixe_numero(&code_reference));
    let mut bilan = Bilan {
        absents: manquants.len(),
        ..Bilan::default()
    };
    let cardinfo = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db())?;

    // Seuls les noms introuvables tels quels coûtent une requête.
    let a_resoudre = noms_a_resoudre(&cardinfo, &manquants)?;
    let fiches = if a_resoudre.is_empty() {
        HashMap::new()
    } else {
        yugipedia::fiches(client, &a_resoudre).await?
    };
    let (decide, a_ecrire) = decider(&cardinfo, &manquants, &fiches, &set_name, raretes)?;
    bilan.ajouts = decide.ajouts;
    bilan.introuvables = decide.introuvables;
    for a in &bilan.ajouts {
        tracing::info!(classeur = %code, numero = %a.numero, carte = %a.nom, lignes = a.lignes,
            "numéro absent retrouvé");
    }
    for (numero, nom) in &bilan.introuvables {
        tracing::warn!(classeur = %code, numero = %numero, nom = %nom,
            "numéro absent : carte introuvable dans la base, rien n'est créé");
    }

    if !ecrire {
        return Ok(Issue::Fait(bilan));
    }
    let conn = {
        let mut conn = Connection::open(paths.classeur_db(code))?;
        if !a_ecrire.is_empty() {
            ecrire_lignes(&mut conn, &a_ecrire)?;
        }
        conn
    };

    // L'image de chaque tirage en cadre normal, pour les numéros que la base
    // ne connaît pas — ceux ajoutés maintenant, et ceux d'un passage
    // précédent. Yugipedia en a une, sous le nom de la fiche ; les tirages en
    // variante, eux, sont servis par la passe artworks qui suit.
    let orphelines = lignes_orphelines(&conn, &cardinfo)?;
    if orphelines.is_empty() {
        return Ok(Issue::Fait(bilan));
    }
    let noms = noms_par_numero(&entrees);
    let mut a_chercher: Vec<String> = orphelines
        .iter()
        .filter_map(|(numero, _)| noms.get(numero).cloned())
        .collect();
    a_chercher.sort();
    a_chercher.dedup();
    match artwork::fichiers_pour_set(client, &code_reference, &a_chercher, &langue).await {
        Ok(index) => {
            let poses = images_normales(&orphelines, &noms, &index);
            bilan.images = poser_images(&conn, &poses)?;
            if bilan.images > 0 {
                tracing::info!(classeur = %code, lignes = bilan.images,
                    "images de tirage posées sur les numéros absents de la base");
            }
        }
        Err(e) => tracing::warn!(classeur = %code, erreur = %e,
            "images des numéros absents de la base non cherchées — l'illustration de la base reste"),
    }
    Ok(Issue::Fait(bilan))
}

/// L'image Yugipedia de chaque ligne ajoutée en cadre normal — fonction
/// **pure**.
///
/// `lignes` : `(numéro, rareté)` des lignes en cadre normal à servir. Rend
/// `(numéro, rareté, url)`. Seul un fichier **sans suffixe de variante**, de
/// la bonne rareté, et **unique**, est retenu : deux candidats, on ne choisit
/// pas ; aucun, la ligne garde l'illustration de la base.
#[must_use]
pub fn images_normales(
    lignes: &[(String, String)],
    noms_set_list: &HashMap<String, String>,
    index: &ygo_sources::yugipedia::artwork::IndexFichiers,
) -> Vec<(String, String, String)> {
    use ygo_sources::yugipedia::artwork::{abbr_rarete, cle_comparaison};
    let mut poses = Vec::new();
    for (numero, rarete) in lignes {
        let Some(nom) = noms_set_list.get(numero) else {
            continue;
        };
        let cle = (
            cle_comparaison(nom),
            abbr_rarete(rarete).to_ascii_uppercase(),
        );
        let mut urls: Vec<&str> = index
            .get(&cle)
            .map(|v| {
                v.iter()
                    .filter(|c| c.infos.variante.is_empty())
                    .map(|c| c.card_url.as_str())
                    .collect()
            })
            .unwrap_or_default();
        urls.sort_unstable();
        urls.dedup();
        if let [url] = urls.as_slice() {
            let pose = (numero.clone(), rarete.clone(), (*url).to_owned());
            if !poses.contains(&pose) {
                poses.push(pose);
            }
        }
    }
    poses
}

/// Le nom que la Set list donne à chaque numéro, numéro en majuscules.
fn noms_par_numero(entrees: &[EntreeSetList]) -> HashMap<String, String> {
    let mut noms = HashMap::new();
    for e in entrees {
        let nom = e.nom.trim();
        if !nom.is_empty() {
            noms.entry(e.numero.trim().to_uppercase())
                .or_insert_with(|| nom.to_owned());
        }
    }
    noms
}

/// Les lignes en cadre normal dont le numéro n'a **aucun** tirage dans
/// `cardinfo.db`, et qui n'ont pas encore d'image Yugipedia — `(numéro,
/// rareté)`, sans doublon.
///
/// Ce sont les numéros que ce module a ajoutés : la base ne leur connaît pas
/// d'image de tirage, `images_tirage` n'a donc rien pu leur poser.
fn lignes_orphelines(
    classeur: &Connection,
    cardinfo: &Connection,
) -> Result<Vec<(String, String)>> {
    let ext = if colonne_existe(classeur, "extended_art")? {
        "AND COALESCE(extended_art,0) = 0"
    } else {
        ""
    };
    let lignes: Vec<(String, String)> = classeur
        .prepare(&format!(
            "SELECT DISTINCT UPPER(TRIM(set_code)), rarity FROM cards \
              WHERE set_code IS NOT NULL AND rarity IS NOT NULL {ext} \
                AND COALESCE(card_image_url,'') NOT LIKE '%yugipedia.com%'"
        ))?
        .query_map([], |l| Ok((l.get(0)?, l.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut connu = cardinfo.prepare("SELECT 1 FROM set_prints WHERE set_code = ?1 LIMIT 1")?;
    let mut orphelines = Vec::new();
    for (numero, rarete) in lignes {
        if !connu.exists([&numero])? {
            orphelines.push((numero, rarete));
        }
    }
    Ok(orphelines)
}

/// Écrit les images décidées par [`images_normales`], sur les seules lignes
/// en cadre normal du numéro et de la rareté.
fn poser_images(conn: &Connection, poses: &[(String, String, String)]) -> Result<usize> {
    let ext = if colonne_existe(conn, "extended_art")? {
        "AND COALESCE(extended_art,0) = 0"
    } else {
        ""
    };
    let mut maj = conn.prepare(&format!(
        "UPDATE cards SET card_image_url = ?3 \
          WHERE set_code = ?1 AND rarity = ?2 {ext} \
            AND COALESCE(card_image_url,'') NOT LIKE '%yugipedia.com%'"
    ))?;
    let mut n = 0;
    for (numero, rarete, url) in poses {
        n += maj.execute((numero, rarete, url))?;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn entree(numero: &str, nom: &str, raretes: &[&str], ext: bool) -> EntreeSetList {
        EntreeSetList {
            numero: numero.to_owned(),
            nom: nom.to_owned(),
            raretes: raretes.iter().map(|r| (*r).to_owned()).collect(),
            extended_art: ext,
            note: String::new(),
        }
    }

    /// Le cas réel : deux lignes de Set list pour `LOCH-JP013`, cadre normal
    /// puis Overframe ; les numéros présents et ceux d'une autre région ne
    /// sont pas rendus.
    #[test]
    fn seuls_les_numeros_absents_du_prefixe_sont_rendus() {
        let nom = "Odd-Eyes Pendulum Dragon, Four Heavenly Dragons";
        let entrees = [
            entree("LOCH-JP012", "Gagaga Girl", &["Ultra Rare"], false),
            entree("LOCH-JP013", nom, &["Ultra Rare", "Secret Rare"], false),
            entree(
                "LOCH-JP013",
                nom,
                &["Ultra Rare", "Prismatic Secret Rare", "Grand Master Rare"],
                true,
            ),
            entree("LOCH-SC013", nom, &["Ultra Rare"], false),
        ];
        let presents: HashSet<String> = ["LOCH-JP012".to_owned()].into();
        let a = absents(&entrees, &presents, "LOCH-JP");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].numero, "LOCH-JP013");
        assert_eq!(a[0].nom, nom);
        assert_eq!(
            a[0].tirages,
            [
                ("Ultra Rare".to_owned(), 0),
                ("Secret Rare".to_owned(), 0),
                ("Ultra Rare".to_owned(), 1),
                ("Prismatic Secret Rare".to_owned(), 1),
                ("Grand Master Rare".to_owned(), 1),
            ]
        );
    }

    #[test]
    fn une_entree_sans_rarete_ou_sans_nom_ne_cree_rien() {
        let entrees = [
            entree("LOCH-JP001", "Carte", &[], false),
            entree("LOCH-JP002", "", &["Ultra Rare"], false),
        ];
        assert!(absents(&entrees, &HashSet::new(), "LOCH-JP").is_empty());
    }

    fn cardinfo() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE cards (uuid TEXT, ygoprodeck_id INTEGER, card_type TEXT, atk INTEGER, \
                                 def INTEGER, level INTEGER, attribute TEXT, race TEXT);
             CREATE TABLE card_texts (card_uuid TEXT, language TEXT, name TEXT, effect TEXT);
             CREATE TABLE card_images (uuid TEXT, card_uuid TEXT, ygoprodeck_image_id INTEGER, \
                                       art_url TEXT, card_url TEXT);
             INSERT INTO cards VALUES ('oe', 75787708, 'monster', 2500, 2000, 7, 'DARK', 'Dragon');
             INSERT INTO card_texts VALUES ('oe', 'en', \
                 'Odd-Eyes Pendulum Dragon of the Four Heavenly Dragons', '');
             INSERT INTO card_texts VALUES ('oe', 'ja', '四天の龍', '');
             INSERT INTO card_images VALUES ('img-op', 'oe', NULL, '', 'https://ms.yugipedia.com/OP.png');
             INSERT INTO card_images VALUES ('img-ygo', 'oe', 75787708, \
                 'https://images.ygoprodeck.com/images/cards_cropped/75787708.jpg', \
                 'https://images.ygoprodeck.com/images/cards/75787708.jpg');",
        )
        .unwrap();
        c
    }

    fn priorites() -> Priorites {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("rarity_config.json");
        std::fs::write(&chemin, r#"{"Ultra Rare": 4, "Secret Rare": 5}"#).unwrap();
        Priorites::charger(&chemin)
    }

    /// Le nom officiel est retrouvé sans tenir compte de la casse ; un nom
    /// porté par deux cartes, ou par aucune, ne désigne rien.
    #[test]
    fn la_carte_se_retrouve_par_son_nom_exact_et_unique() {
        let c = cardinfo();
        assert_eq!(
            carte_par_nom(&c, "odd-eyes pendulum dragon of the four heavenly dragons").unwrap(),
            Some("oe".to_owned())
        );
        assert_eq!(
            carte_par_nom(&c, "Odd-Eyes Pendulum Dragon, Four Heavenly Dragons").unwrap(),
            None,
            "l'ancien nom n'est pas dans la base : c'est Yugipedia qui le résout"
        );
        c.execute(
            "INSERT INTO card_texts VALUES ('autre', 'en', \
             'Odd-Eyes Pendulum Dragon of the Four Heavenly Dragons', '')",
            (),
        )
        .unwrap();
        assert_eq!(
            carte_par_nom(&c, "Odd-Eyes Pendulum Dragon of the Four Heavenly Dragons").unwrap(),
            None,
            "deux cartes du même nom : on ne choisit pas"
        );
    }

    /// Le cas réel du 2026-10-10 : la fiche Yugipedia porte le nom de la Set
    /// list, la base l'autre nom — c'est le password qui fait le pont. Sans
    /// fiche, ou sans password, rien n'est créé.
    #[test]
    fn le_password_de_la_fiche_retrouve_la_carte_quand_aucun_nom_ne_colle() {
        let c = cardinfo();
        let nom = "Odd-Eyes Pendulum Dragon, Four Heavenly Dragons";
        let absent = Absent {
            numero: "LOCH-JP013".to_owned(),
            nom: nom.to_owned(),
            tirages: vec![("Ultra Rare".to_owned(), 0)],
        };
        let manquants = [absent];
        assert_eq!(noms_a_resoudre(&c, &manquants).unwrap(), [nom]);

        let fiche = |password| {
            HashMap::from([(
                nom.to_owned(),
                Fiche {
                    titre: nom.to_owned(),
                    password,
                },
            )])
        };
        let (bilan, lignes) =
            decider(&c, &manquants, &fiche(Some(75_787_708)), "S", &priorites()).unwrap();
        assert_eq!(bilan.ajouts.len(), 1);
        assert_eq!(bilan.ajouts[0].via, Rapprochement::Password);
        assert_eq!(
            bilan.ajouts[0].nom,
            "Odd-Eyes Pendulum Dragon of the Four Heavenly Dragons"
        );
        assert_eq!(lignes.len(), 1);

        let (bilan, lignes) = decider(&c, &manquants, &fiche(None), "S", &priorites()).unwrap();
        assert!(lignes.is_empty());
        assert_eq!(
            bilan.introuvables,
            [("LOCH-JP013".to_owned(), nom.to_owned())]
        );

        let (bilan, _) = decider(&c, &manquants, &HashMap::new(), "S", &priorites()).unwrap();
        assert_eq!(bilan.introuvables.len(), 1, "sans fiche, rien");
    }

    /// Une ligne par tirage, l'illustration YGOPRODeck, pas de nom français
    /// inventé, l'Overframe gardé.
    #[test]
    fn les_lignes_portent_la_carte_de_la_base_et_rien_d_invente() {
        let c = cardinfo();
        let absent = Absent {
            numero: "LOCH-JP013".to_owned(),
            nom: "Odd-Eyes Pendulum Dragon, Four Heavenly Dragons".to_owned(),
            tirages: vec![
                ("Ultra Rare".to_owned(), 0),
                ("Grand Master Rare".to_owned(), 1),
            ],
        };
        let (nom, lignes) = lignes_de(
            &c,
            "oe",
            &absent,
            "Limit Over Collection: The Heroes",
            &priorites(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(nom, "Odd-Eyes Pendulum Dragon of the Four Heavenly Dragons");
        assert_eq!(lignes.len(), 2);
        let l = &lignes[0];
        assert_eq!(l.set_code, "LOCH-JP013");
        assert_eq!(l.card_image_id, Some(75_787_708));
        assert_eq!(l.card_image_uuid, "img-ygo");
        assert_eq!(
            (l.atk, l.def_val, l.level),
            (Some(2500), Some(2000), Some(7))
        );
        assert!(
            l.name_fr.is_empty(),
            "aucun nom français connu : aucun écrit"
        );
        assert_eq!(lignes[1].extended_art, 1);
        assert_eq!(lignes[1].rarity, "Grand Master Rare");
    }

    fn candidat(
        cle: &str,
        abbr: &str,
        variante: &str,
        url: &str,
    ) -> ygo_sources::yugipedia::artwork::Candidat {
        ygo_sources::yugipedia::artwork::Candidat {
            segment: cle.to_owned(),
            cle_carte: cle.to_owned(),
            infos: ygo_sources::yugipedia::artwork::InfosFichier {
                fichier: url.to_owned(),
                langue: "JP".to_owned(),
                rarete_abbr: abbr.to_owned(),
                edition: String::new(),
                variante: variante.to_owned(),
            },
            card_url: url.to_owned(),
            image_id: -1,
            uuid: format!("yugipedia:{url}"),
        }
    }

    /// Les fichiers portent le nom de la fiche, pas celui de la base : c'est
    /// le nom de la Set list qui les retrouve. Seul un fichier nu et unique de
    /// la bonne rareté est posé.
    #[test]
    fn une_ligne_normale_recoit_le_fichier_nu_de_sa_rarete() {
        let cle = "oddeyespendulumdragonfourheavenlydragons";
        let mut index = ygo_sources::yugipedia::artwork::IndexFichiers::new();
        index.insert(
            (cle.to_owned(), "UR".to_owned()),
            vec![
                candidat(cle, "UR", "", "OE-UR.png"),
                candidat(cle, "UR", "EA", "OE-UR-EA.png"),
            ],
        );
        index.insert(
            (cle.to_owned(), "SCR".to_owned()),
            vec![
                candidat(cle, "ScR", "", "A.png"),
                candidat(cle, "ScR", "", "B.png"),
            ],
        );
        let noms = HashMap::from([(
            "LOCH-JP013".to_owned(),
            "Odd-Eyes Pendulum Dragon, Four Heavenly Dragons".to_owned(),
        )]);
        let lignes = [
            ("LOCH-JP013".to_owned(), "Ultra Rare".to_owned()),
            ("LOCH-JP013".to_owned(), "Secret Rare".to_owned()),
        ];
        let poses = images_normales(&lignes, &noms, &index);
        assert_eq!(
            poses,
            [(
                "LOCH-JP013".to_owned(),
                "Ultra Rare".to_owned(),
                "OE-UR.png".to_owned()
            )],
            "UR : le fichier nu ; ScR : deux candidats, on ne choisit pas"
        );
    }

    /// Écrites dans un vrai classeur, les lignes prennent leur place entre
    /// leurs voisines ; l'ordre du reste ne bouge pas.
    #[test]
    fn le_numero_ajoute_prend_sa_place_entre_ses_voisins() {
        let mut cl = Connection::open_in_memory().unwrap();
        cl.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
        for (code, rang) in [("LOCH-JP012", 0), ("LOCH-JP014", 1)] {
            cl.execute(
                "INSERT INTO cards (set_code, rarity, name, sort_order, possessed, quantite) \
                 VALUES (?1, 'Ultra Rare', 'x', ?2, 0, 0)",
                rusqlite::params![code, rang],
            )
            .unwrap();
        }
        let c = cardinfo();
        let absent = Absent {
            numero: "LOCH-JP013".to_owned(),
            nom: "n".to_owned(),
            tirages: vec![("Ultra Rare".to_owned(), 0)],
        };
        let (_, lignes) = lignes_de(&c, "oe", &absent, "S", &priorites())
            .unwrap()
            .unwrap();
        assert_eq!(ecrire_lignes(&mut cl, &lignes).unwrap(), 1);
        let ordre: Vec<String> = cl
            .prepare("SELECT set_code FROM cards ORDER BY sort_order")
            .unwrap()
            .query_map([], |l| l.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(ordre, ["LOCH-JP012", "LOCH-JP013", "LOCH-JP014"]);
        let rangs: Vec<i64> = cl
            .prepare("SELECT sort_order FROM cards ORDER BY sort_order")
            .unwrap()
            .query_map([], |l| l.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            rangs,
            [0, 1, 2],
            "le suivant a reculé d'un cran, rien d'autre"
        );
        let possedee: i64 = cl
            .query_row(
                "SELECT possessed + quantite FROM cards WHERE set_code = 'LOCH-JP013'",
                [],
                |l| l.get(0),
            )
            .unwrap();
        assert_eq!(possedee, 0, "une ligne ajoutée naît non possédée");
    }
}

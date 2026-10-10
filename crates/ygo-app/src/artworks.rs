// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Passe artworks — aligner le classeur sur la Set list Yugipedia.
//!
//! Portage de `completer_artworks_variantes` et de ses aides
//! (`creation_classeur_service.py`, ~370 lignes).
//!
//! # Qui fait autorité sur quoi
//!
//! C'est la clé de voûte du module, et elle tient en trois lignes :
//!
//! | source | autorité sur |
//! |---|---|
//! | Set list Yugipedia | la **structure** — quels numéros, quelles raretés, lesquels sont en variante |
//! | YGOPRODeck / YGOJSON | les **métadonnées** et les illustrations normales |
//! | fichiers Yugipedia | les **images des seuls tirages en variante** |
//!
//! La passe ne crée jamais un numéro que le classeur ignore : les fabriquer
//! demanderait des statistiques et un nom français qu'aucune source ne donne
//! pour eux. Ils sont comptés et journalisés.
//!
//! # Le piège central : un fichier sans suffixe
//!
//! Yugipedia n'ajoute `-AA` ou `-EA` que pour **désambiguïser** deux fichiers
//! homonymes. Un tirage qui n'existe qu'en variante porte donc un nom tout à
//! fait ordinaire. Prendre « fichier sans suffixe » pour « illustration
//! d'origine » ferait manquer `RA05-EN083` ; prendre l'inverse poserait sur la
//! ligne variante **exactement la même image** que sur la ligne normale — le
//! seul résultat qu'il faut absolument éviter, parce qu'il est invisible à la
//! relecture du code et criant à l'écran.
//!
//! D'où la règle à trois niveaux de [`choisir_fichier`], et les deux conditions
//! qui gouvernent `accepter_nus` dans [`planifier`].
//!
//! # Ce que la passe ne descend jamais
//!
//! `extended_art` **monte** de 0 à 1 quand la référence le déclare, jamais
//! l'inverse. Un tirage marqué Overframe par une passe antérieure, ou par
//! l'utilisateur, ne se fait pas déclasser par une Set list incomplète.
//!
//! # Ce qui garde ce module honnête
//!
//! L'oracle `LOCR-JP` ancre la passe de bout en bout mais discrimine mal : sur
//! ce set, presque toutes les branches donnent le même compte (cf. l'en-tête de
//! `tests/oracle_artworks.rs`). Ce sont les tests unitaires ci-dessous qui
//! tiennent les branches. Ils ont été calibrés par mutation — dix altérations
//! du planificateur, dix détectées :
//!
//! | mutation | tuée par |
//! |---|---|
//! | `accepter_nus` toujours vrai sur la rareté visée | `un_fichier_nu_est_refuse_quand_le_meme_couple_a_un_tirage_normal` |
//! | `accepter_nus` toujours faux sur la rareté visée | `un_fichier_nu_illustre_une_variante_quand_le_tirage_n_existe_qu_en_variante` |
//! | repli acceptant toujours les nus | `le_repli_refuse_ce_meme_nu_si_le_numero_a_un_tirage_normal` |
//! | repli refusant toujours les nus | `le_repli_n_accepte_un_nu_que_si_le_numero_est_tout_variante` |
//! | famille du slot ignorée | `la_famille_du_slot_passe_avant_tout_autre_suffixe` |
//! | nu essayé avant les suffixés | `a_defaut_de_la_bonne_famille_n_importe_quel_suffixe` |
//! | retenue des fichiers levée | `un_fichier_ne_sert_jamais_deux_fois` |
//! | appariement trié sur le seul `rowid` | `les_lignes_sont_appariees_cadre_normal_d_abord` |
//! | drapeau `extended_art` redescendant | `le_drapeau_monte_mais_ne_descend_jamais` |
//! | numéro absent créé au lieu d'être compté | `un_numero_absent_du_classeur_est_compte_et_non_cree` |
//!
//! L'applicateur a passé le même examen — sept mutations, sept détectées :
//!
//! | mutation | tuée par |
//! |---|---|
//! | garde d'idempotence retirée | `la_garde_d_idempotence_epargne_une_ligne_deja_servie` |
//! | ligne créée née possédée | `une_ligne_creee_herite_des_metadonnees_et_nait_non_possedee` |
//! | rareté imposée ignorée à l'insertion | `une_ligne_creee_herite_des_metadonnees_et_nait_non_possedee` |
//! | héritage pris sur n'importe quelle rareté | `la_ligne_source_est_celle_de_la_meme_rarete_quand_elle_existe` |
//! | création ratée décalant les indices suivants | `sans_ligne_du_numero_rien_ne_nait_et_l_illustration_ne_se_perd_pas` |
//! | drapeau redescendant à l'écriture | `le_drapeau_ne_remonte_pas_une_ligne_deja_marquee` |
//! | inventaire gardant les lignes sans nom | `l_inventaire_ecarte_les_lignes_sans_nom_et_majuscule_les_numeros` |

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};

use ygo_core::rarity::Priorites;

use crate::error::Result;

/// Une ligne du classeur, réduite à ce que la passe consulte.
///
/// Le `rowid` est indispensable : deux lignes peuvent partager le même couple
/// (numéro, rareté) — la normale et l'Overframe sur les sets OCG — et il faut
/// pouvoir viser l'une sans toucher l'autre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneInventaire {
    /// Identifiant de la ligne dans la table `cards`.
    pub rowid: i64,
    /// Nom de la carte.
    pub name: String,
    /// Code de set, en majuscules.
    pub set_code: String,
    /// Libellé de rareté, tel qu'il est écrit dans le classeur.
    pub rarity: String,
    /// 1 si Overframe.
    pub ext: i64,
}

/// Un fichier Yugipedia candidat, tel que la passe le manipule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FichierCandidat {
    /// Nom du fichier — sert aussi de clé d'unicité.
    pub fichier: String,
    /// Clé de comparaison du nom de carte.
    pub cle_carte: String,
    /// Abréviation de rareté lue dans le nom, en majuscules.
    pub rarete_abbr: String,
    /// Suffixe de variante (`""`, `AA`, `EA`…).
    pub variante: String,
    /// URL de l'image.
    pub card_url: String,
    /// Identifiant synthétique, négatif.
    pub image_id: i64,
}

impl FichierCandidat {
    /// Le fichier appartient-il à la famille « extended art » ?
    pub fn est_extended_art(&self) -> bool {
        self.variante.to_ascii_uppercase().starts_with("EA")
    }
}

/// Ce que la passe demande d'écrire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Créer une ligne manquante pour atteindre le compte de la référence.
    Inserer {
        /// Rang de la ligne créée, pour que les actions suivantes la visent.
        indice: usize,
        /// Numéro de collection.
        numero: String,
        /// Libellé de rareté à écrire.
        rarete: String,
        /// 1 si le slot est déclaré extended art.
        extended_art: i64,
    },
    /// Monter `extended_art` à 1 sur une ligne existante.
    Flaguer {
        /// Ligne visée.
        rowid: i64,
    },
    /// Poser une illustration Yugipedia sur une ligne.
    PoserIllustration {
        /// Ligne visée — existante ou fraîchement créée.
        cible: Cible,
        /// Fichier retenu.
        fichier: FichierCandidat,
        /// `Some(1)` quand le slot est extended art, `None` sinon : le Python
        /// ne touche au drapeau que dans ce cas.
        extended_art: Option<i64>,
    },
}

/// Ce qu'une action vise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cible {
    /// Une ligne déjà présente au classeur.
    Existante(i64),
    /// Une ligne créée par cette même passe, désignée par son rang de création.
    Nouvelle(usize),
}

/// Ce que la passe a changé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bilan {
    /// Lignes créées.
    pub ajoutes: usize,
    /// Illustrations posées.
    pub illustrations: usize,
    /// Drapeaux `extended_art` montés.
    pub flags: usize,
    /// Illustrations empruntées à une autre rareté de la même carte.
    pub reutilisees: usize,
    /// Numéros que la Set list connaît et que le classeur ignore.
    pub absents: usize,
}

/// Clé de comparaison d'un libellé de rareté.
///
/// Réexport de [`ygo_sources::yugipedia::structure::normaliser_rarete`], qui
/// porte `structure_yugipedia.normaliser_rarete`. Le classeur et la Set list
/// doivent employer **la même** clé : deux implémentations, même identiques le
/// jour où on les écrit, finiraient par diverger sur un cas limite — et la
/// divergence se manifesterait par des lignes qui ne s'apparient plus, sans
/// erreur ni message.
pub use ygo_sources::yugipedia::structure::normaliser_rarete;

/// La forme canonique d'un libellé de rareté, ou le libellé tel quel s'il
/// n'est reconnu par aucune table.
///
/// On ne devine pas : `force-SMW` reste `force-SMW`.
fn canonique(libelle: &str, raretes: &Priorites) -> String {
    ygo_core::rarity::canon::canoniser(libelle, raretes)
        .map_or_else(|| libelle.to_owned(), |c| c.libelle)
}

/// Choisit l'image la plus adaptée à un slot de variante.
///
/// Portage de `_choisir_fichier`. Trois niveaux de préférence, dans l'ordre :
///
/// 1. un fichier **de la même famille** que le slot — suffixe `EA` pour un slot
///    extended art, suffixe non-`EA` sinon ;
/// 2. n'importe quel fichier **suffixé** ;
/// 3. un fichier **sans suffixe** — et seulement si `accepter_nus`.
///
/// # Quand accepter un fichier nu
///
/// Vrai pour les fichiers de la **bonne rareté** : Yugipedia n'ajoute un
/// suffixe que pour désambiguïser, donc un tirage qui n'existe qu'en variante
/// porte un nom ordinaire.
///
/// Faux en repli sur une **autre rareté** : un fichier nu emprunté à une rareté
/// voisine pourrait être l'illustration d'origine, et on afficherait alors sur
/// la ligne « variante » la même image que sur la ligne normale.
pub fn choisir_fichier<'a>(
    candidats: &'a [FichierCandidat],
    variante: &str,
    utilises: &HashSet<String>,
    accepter_nus: bool,
) -> Option<&'a FichierCandidat> {
    let libres: Vec<&FichierCandidat> = candidats
        .iter()
        .filter(|c| !utilises.contains(&c.fichier))
        .collect();
    if libres.is_empty() {
        return None;
    }
    let veut_ea = variante == "EA";

    // 1. Même famille que le slot, et suffixé.
    if let Some(c) = libres
        .iter()
        .find(|c| c.est_extended_art() == veut_ea && !c.variante.is_empty())
    {
        return Some(c);
    }
    // 2. N'importe quel suffixé.
    if let Some(c) = libres.iter().find(|c| !c.variante.is_empty()) {
        return Some(c);
    }
    // 3. Un fichier nu, si on l'accepte.
    if accepter_nus {
        return libres.first().copied();
    }
    None
}

/// La structure de référence : `(numéro, rareté normalisée)` vers la liste de
/// ses variantes, normales d'abord.
pub type Reference = BTreeMap<(String, String), Vec<String>>;

/// Les images du set, indexées par `(clé de carte, rareté en MAJUSCULES)`.
pub type IndexCandidats = HashMap<(String, String), Vec<FichierCandidat>>;

/// Les illustrations à poser, par nom de fichier.
pub type IndexIllustrations = HashMap<String, Illustration>;

/// Planifie la passe, sans rien écrire.
///
/// Portage de `completer_artworks_variantes`, moins ses écritures : la fonction
/// rend la liste des actions et le bilan, ce qui la rend éprouvable sans base
/// ni réseau. C'est le découpage que le cahier des charges demande et que le
/// Python ne fait pas.
///
/// - `reference` : ce que dit la Set list ;
/// - `inventaire` : les lignes du classeur ;
/// - `fichiers` : les images Yugipedia du set, indexées par
///   `(clé de carte, abréviation de rareté en majuscules)` ;
/// - `libelles_reference` : le libellé de rareté que la Set list emploie, pour
///   les raretés que le classeur ne connaît pas encore ;
/// - `abbr_rarete` et `cle_comparaison` : passées en paramètre pour que ce
///   module ne dépende pas de `ygo-sources` ;
/// - `raretes` : la liste des Options, qui sert à **canoniser** les libellés
///   avant de les comparer (voir plus bas).
///
/// # La comparaison de raretés est canonisée — ÉCART DÉLIBÉRÉ AU PORTAGE
///
/// Le Python compare avec `normaliser_rarete` seul : minuscules, caractères
/// non alphanumériques ôtés. Cette clé ne rapproche **jamais** `PlScR` de
/// `Platinum Secret Rare`. Or Yugipedia écrit la rareté en abrégé sur certains
/// sets et en toutes lettres sur d'autres — `RA02` en abrégé, `LOCR-JP` en
/// entier. Sur un set abrégé, la passe ne retrouve donc aucune ligne, conclut
/// que le tirage manque, et **l'insère**.
///
/// Le dégât est mesuré sur les bases réelles : **751 lignes en double** — 567
/// sur `RA02`, 132 sur `LDK2`, 36 sur `SDLI`, 8 sur `EGO1`, 8 sur `EGS1` —
/// toutes à quantité nulle, `sort_order` recopié de la première ligne du
/// groupe, `edition` vide.
///
/// [`ygo_core::rarity::canon::cle`] referme le trou des deux côtés. L'oracle
/// n'en souffre pas : `LOCH-JP` et `LOCR-JP`, les deux sets figés, écrivent
/// leurs raretés en toutes lettres — la canonisation y est l'identité, et les
/// tests le prouvent en restant verts.
pub fn planifier(
    reference: &Reference,
    libelles_reference: &HashMap<String, String>,
    inventaire: &[LigneInventaire],
    fichiers: &IndexCandidats,
    abbr_rarete: &dyn Fn(&str) -> String,
    cle_comparaison: &dyn Fn(&str) -> String,
    raretes: &Priorites,
) -> (Vec<Action>, Bilan) {
    let mut actions = Vec::new();
    let mut bilan = Bilan::default();
    let cle_rarete = |libelle: &str| ygo_core::rarity::canon::cle(libelle, raretes);

    // Index du classeur, et orthographe de rareté réellement employée.
    let mut index: HashMap<(String, String), Vec<&LigneInventaire>> = HashMap::new();
    let mut orthographe: HashMap<String, String> = HashMap::new();
    let mut noms: HashMap<String, String> = HashMap::new();
    for l in inventaire {
        let cle_r = cle_rarete(&l.rarity);
        index
            .entry((l.set_code.clone(), cle_r.clone()))
            .or_default()
            .push(l);
        // L'orthographe retenue pour d'éventuelles lignes nouvelles est la
        // forme canonique du libellé local : un classeur qui porte encore
        // `PLatinum Secret Rare` ne doit pas propager sa coquille.
        orthographe
            .entry(cle_r)
            .or_insert_with(|| canonique(&l.rarity, raretes));
        noms.entry(l.set_code.clone())
            .or_insert_with(|| l.name.clone());
    }
    let numeros_classeur: HashSet<&String> = index.keys().map(|(sc, _)| sc).collect();

    // La Set list est réindexée sur la clé canonique, elle aussi : c'est le
    // second bout de la comparaison. Deux libellés du wiki qui désignent la
    // même rareté fusionnent ici — ils décrivent alors les mêmes tirages, et
    // l'ordre « normales d'abord » est rétabli après fusion.
    let reference: Reference = {
        let mut fusionnee: Reference = BTreeMap::new();
        for ((numero, cle_r), variantes) in reference {
            let libelle = libelles_reference.get(cle_r).unwrap_or(cle_r);
            fusionnee
                .entry((numero.clone(), cle_rarete(libelle)))
                .or_default()
                .extend(variantes.iter().cloned());
        }
        for variantes in fusionnee.values_mut() {
            variantes.sort_by(|a, b| {
                (usize::from(!a.is_empty()), a).cmp(&(usize::from(!b.is_empty()), b))
            });
        }
        fusionnee
    };
    let libelles_reference: HashMap<String, String> = libelles_reference
        .values()
        .map(|libelle| (cle_rarete(libelle), canonique(libelle, raretes)))
        .collect();
    let reference: &Reference = &reference;
    let libelles_reference = &libelles_reference;

    // Toutes les images d'une carte, toutes raretés confondues : repli quand la
    // rareté visée n'a aucun fichier dédié. Deux raretés d'une même variante
    // montrent la MÊME illustration — seul le foil change — donc emprunter est
    // correct, et infiniment préférable à laisser l'illustration d'origine sur
    // une ligne « variante ».
    let mut pool_par_carte: HashMap<String, Vec<FichierCandidat>> = HashMap::new();
    for ((cle_c, _), infos) in fichiers {
        pool_par_carte
            .entry(cle_c.clone())
            .or_default()
            .extend(infos.iter().cloned());
    }

    // Numéros dont TOUS les tirages sont des variantes : aucun fichier de la
    // carte ne peut y être l'illustration d'origine, donc même un fichier nu
    // emprunté à une autre rareté est sûr (cas `RA05-EN083`, `RA05-EN141`).
    let mut tout_variante: BTreeSet<&String> = BTreeSet::new();
    let mut pas_tout_variante: BTreeSet<&String> = BTreeSet::new();
    for ((numero, _), variantes) in reference {
        if variantes.iter().all(|v| !v.is_empty()) {
            tout_variante.insert(numero);
        } else {
            pas_tout_variante.insert(numero);
        }
    }
    // Le Python parcourt dans l'ordre et `discard` : un numéro qui a au moins
    // un couple non-tout-variante finit hors de l'ensemble.
    for numero in &pas_tout_variante {
        tout_variante.remove(*numero);
    }

    let mut absents: BTreeSet<&String> = BTreeSet::new();
    let mut cree = 0usize;

    for ((numero, cle_r), variantes) in reference {
        if !numeros_classeur.contains(numero) {
            absents.insert(numero);
            continue;
        }

        // Normales d'abord, puis Overframe ; à cadre égal, ordre d'insertion.
        let mut triees: Vec<&LigneInventaire> = index
            .get(&(numero.clone(), cle_r.clone()))
            .cloned()
            .unwrap_or_default();
        triees.sort_by_key(|l| (l.ext, l.rowid));
        let mut rows: Vec<Cible> = triees.iter().map(|l| Cible::Existante(l.rowid)).collect();
        let mut cadres: Vec<i64> = triees.iter().map(|l| l.ext).collect();

        let nom = noms.get(numero).cloned().unwrap_or_default();
        let rarete = orthographe
            .get(cle_r)
            .cloned()
            .or_else(|| libelles_reference.get(cle_r).cloned())
            .unwrap_or_default();

        // ── Compléter le nombre de tirages ───────────────────────────────
        for v in variantes.iter().skip(rows.len()) {
            let ext = i64::from(v == "EA");
            actions.push(Action::Inserer {
                indice: cree,
                numero: numero.clone(),
                rarete: rarete.clone(),
                extended_art: ext,
            });
            rows.push(Cible::Nouvelle(cree));
            cadres.push(ext);
            cree += 1;
            bilan.ajoutes += 1;
        }

        // ── Illustrations et drapeaux, appariés par rang ─────────────────
        let cle_carte = cle_comparaison(&nom);
        let abbr = abbr_rarete(&rarete).to_ascii_uppercase();
        let candidats: Vec<FichierCandidat> = fichiers
            .get(&(cle_carte.clone(), abbr))
            .cloned()
            .unwrap_or_default();
        let deja: HashSet<&String> = candidats.iter().map(|c| &c.fichier).collect();
        let repli: Vec<FichierCandidat> = pool_par_carte
            .get(&cle_carte)
            .map(|v| {
                v.iter()
                    .filter(|c| !deja.contains(&c.fichier))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let numero_tout_variante = tout_variante.contains(numero);
        // Ce couple (numéro, rareté) n'a-t-il aucun tirage en cadre normal ?
        let sans_tirage_normal = variantes.iter().all(|v| !v.is_empty());

        let mut utilises: HashSet<String> = HashSet::new();
        for (i, v) in variantes.iter().enumerate() {
            let Some(&cible) = rows.get(i) else { break };
            if v == "EA" && cadres.get(i).copied().unwrap_or(0) == 0 {
                if let Cible::Existante(rowid) = cible {
                    actions.push(Action::Flaguer { rowid });
                    bilan.flags += 1;
                }
            }
            if v.is_empty() {
                continue; // slot normal : l'API garde la main
            }

            let mut choisi = choisir_fichier(&candidats, v, &utilises, sans_tirage_normal).cloned();
            if choisi.is_none() && !repli.is_empty() {
                choisi = choisir_fichier(&repli, v, &utilises, numero_tout_variante).cloned();
                if choisi.is_some() {
                    bilan.reutilisees += 1;
                }
            }
            let Some(choisi) = choisi else { continue };
            utilises.insert(choisi.fichier.clone());
            actions.push(Action::PoserIllustration {
                cible,
                fichier: choisi,
                extended_art: (v == "EA").then_some(1),
            });
            bilan.illustrations += 1;
        }
    }

    bilan.absents = absents.len();
    (actions, bilan)
}

// ─────────────────────────────────────────────────────────────────────────────
// Écriture — l'applicateur
// ─────────────────────────────────────────────────────────────────────────────

/// L'illustration telle que la passe l'écrit.
///
/// Réexport structurel de `ygo_sources::yugipedia::artwork::Artwork` : ce
/// module ne dépend pas de `ygo-sources`, il reçoit ses illustrations toutes
/// faites. Les quatre champs sont exactement ceux que le Python pose.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Illustration {
    /// `yugipedia:<fichier>`.
    pub card_image_uuid: String,
    /// Identifiant synthétique, négatif.
    pub card_image_id: i64,
    /// URL pleine résolution.
    pub card_image_url: String,
    /// URL de vignette.
    pub card_image_small: String,
}

/// Lit l'inventaire d'un classeur.
///
/// Portage d'`_inventaire_classeur`. Rend les lignes **et** le `set_name`, qui
/// est le premier non vide rencontré — le classeur peut en avoir des vides.
///
/// Les lignes sans nom sont écartées : ce sont des résidus, et les apparier à
/// un slot poserait une illustration sur une ligne fantôme.
pub fn lire_inventaire(conn: &Connection) -> Result<(Vec<LigneInventaire>, String)> {
    let expression = if colonne_existe(conn, "extended_art")? {
        "COALESCE(extended_art,0)"
    } else {
        "0"
    };
    let sql = format!(
        "SELECT rowid, name, set_code, rarity, set_name, {expression} \
         FROM cards WHERE name IS NOT NULL AND name != '' ORDER BY rowid"
    );
    let mut requete = conn.prepare(&sql)?;
    let lignes: Vec<(LigneInventaire, String)> = requete
        .query_map([], |l| {
            Ok((
                LigneInventaire {
                    rowid: l.get(0)?,
                    name: l.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    set_code: l
                        .get::<_, Option<String>>(2)?
                        .unwrap_or_default()
                        .to_uppercase(),
                    rarity: l.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    ext: l.get::<_, Option<i64>>(5)?.unwrap_or(0),
                },
                l.get::<_, Option<String>>(4)?.unwrap_or_default(),
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;

    let set_name = lignes
        .iter()
        .map(|(_, nom)| nom)
        .find(|nom| !nom.is_empty())
        .cloned()
        .unwrap_or_default();
    Ok((lignes.into_iter().map(|(l, _)| l).collect(), set_name))
}

/// La table `cards` a-t-elle cette colonne ?
///
/// Portage de `_classeur_a_colonne`. Un classeur assez ancien peut n'avoir
/// aucune colonne `extended_art` ; toute la passe doit alors s'en passer sans
/// échouer.
fn colonne_existe(conn: &Connection, colonne: &str) -> Result<bool> {
    Ok(conn
        .prepare("SELECT 1 FROM pragma_table_info('cards') WHERE name = ?1")?
        .exists([colonne])?)
}

/// Applique le plan à un classeur, dans une transaction.
///
/// Les `Action` viennent de [`planifier`] ; les illustrations sont fournies
/// par l'appelant, indexées par nom de fichier — c'est ce qui garde ce module
/// ignorant de `ygo-sources`.
///
/// # Le bilan rendu n'est pas celui du plan
///
/// [`planifier`] compte ce qu'il **prévoit** ; celui-ci compte ce que SQLite a
/// **réellement** modifié. Les deux diffèrent dès que la passe est rejouée :
/// l'`UPDATE` d'illustration porte une garde d'idempotence
/// (`card_image_uuid NOT LIKE 'yugipedia:%'`) et ne touche donc pas une ligne
/// déjà servie. C'est le compte du Python, et c'est celui qui est journalisé.
pub fn appliquer(
    conn: &Connection,
    actions: &[Action],
    illustrations: &IndexIllustrations,
) -> Result<Bilan> {
    let a_extended_art = colonne_existe(conn, "extended_art")?;
    let mut bilan = Bilan::default();
    // Les lignes créées, dans l'ordre de leur indice : une action postérieure
    // les vise par `Cible::Nouvelle`.
    let mut nouvelles: Vec<i64> = Vec::new();

    for action in actions {
        match action {
            Action::Inserer {
                indice,
                numero,
                rarete,
                extended_art,
            } => {
                debug_assert_eq!(*indice, nouvelles.len(), "indices consécutifs");
                match inserer_ligne(conn, numero, rarete, *extended_art, a_extended_art)? {
                    Some(rowid) => {
                        nouvelles.push(rowid);
                        bilan.ajoutes += 1;
                    }
                    // Le Python `continue` : la ligne n'a pas pu naître, faute
                    // d'une ligne du même numéro d'où hériter. Le plan la vise
                    // pourtant encore — d'où le trou, comblé par un rowid
                    // impossible plutôt que par un décalage des suivants.
                    None => nouvelles.push(i64::MIN),
                }
            }
            Action::Flaguer { rowid } => {
                if a_extended_art {
                    bilan.flags += conn.execute(
                        "UPDATE cards SET extended_art = 1 \
                         WHERE rowid = ?1 AND COALESCE(extended_art,0) = 0",
                        [rowid],
                    )?;
                }
            }
            Action::PoserIllustration {
                cible,
                fichier,
                extended_art,
            } => {
                let rowid = match cible {
                    Cible::Existante(rowid) => *rowid,
                    Cible::Nouvelle(indice) => match nouvelles.get(*indice) {
                        Some(&rowid) if rowid != i64::MIN => rowid,
                        _ => continue,
                    },
                };
                let Some(art) = illustrations.get(&fichier.fichier) else {
                    continue;
                };
                bilan.illustrations +=
                    maj_illustration(conn, rowid, art, extended_art.filter(|_| a_extended_art))?;
            }
        }
    }
    Ok(bilan)
}

/// Pose l'illustration sur UNE ligne, ciblée par `rowid`.
///
/// Portage de `_maj_illustration`. La garde
/// `card_image_uuid NOT LIKE 'yugipedia:%'` rend l'opération **idempotente** :
/// rejouer la passe ne retouche pas une ligne déjà servie. Possession,
/// quantité, qualité et édition sont conservées — elles ne figurent pas dans
/// le `SET`.
fn maj_illustration(
    conn: &Connection,
    rowid: i64,
    art: &Illustration,
    extended_art: Option<i64>,
) -> Result<usize> {
    let (fragment, mut parametres): (&str, Vec<Box<dyn rusqlite::ToSql>>) = match extended_art {
        Some(ext) => (", extended_art = ?5", vec![Box::new(ext)]),
        None => ("", Vec::new()),
    };
    let sql = format!(
        "UPDATE cards \
         SET card_image_uuid = ?1, card_image_id = ?2, \
             card_image_url = ?3, card_image_small = ?4{fragment} \
         WHERE rowid = ?{} \
           AND (card_image_uuid IS NULL OR card_image_uuid NOT LIKE 'yugipedia:%')",
        parametres.len() + 5
    );
    let mut tous: Vec<Box<dyn rusqlite::ToSql>> = vec![
        Box::new(art.card_image_uuid.clone()),
        Box::new(art.card_image_id),
        Box::new(art.card_image_url.clone()),
        Box::new(art.card_image_small.clone()),
    ];
    tous.append(&mut parametres);
    tous.push(Box::new(rowid));
    let refs: Vec<&dyn rusqlite::ToSql> = tous.iter().map(AsRef::as_ref).collect();
    Ok(conn.execute(&sql, refs.as_slice())?)
}

/// Colonnes héritées d'une ligne existante par un tirage créé.
const COLONNES_HERITEES: &str = "card_uuid, set_code, rarity, rarity_code, set_name, \
     name, name_fr, card_image_url, card_image_small, card_image_id, sort_order, \
     card_type, atk, def_val, level, attribute, race";

/// Crée un tirage manquant en recopiant une ligne existante du même numéro.
///
/// Portage d'`_inserer_ligne`. Les métadonnées (nom, nom français, nom de set,
/// rang de tri, statistiques) sont héritées d'une ligne de même
/// `(set_code, rareté)` si elle existe, **sinon de n'importe quelle ligne du
/// même `set_code`**. Seules la rareté et le drapeau Overframe sont imposés.
///
/// La carte naît **non possédée** — `possessed = 0`, `quantite = 0`,
/// `qualite = NULL`, `is_custom = 0` —, et son `card_image_uuid` est la chaîne
/// vide, ce qui la laisse éligible à l'`UPDATE` d'illustration qui suit.
///
/// Rend `None` si aucune ligne du numéro n'existe : sans source, rien à
/// hériter, et le Python ne crée rien.
fn inserer_ligne(
    conn: &Connection,
    set_code: &str,
    rarete: &str,
    extended_art: i64,
    a_extended_art: bool,
) -> Result<Option<i64>> {
    // Même rareté d'abord ; à défaut, n'importe quelle ligne du numéro. Chaque
    // requête est rendue optionnelle séparément : un repli ne doit pas masquer
    // une erreur SQL de la première.
    let mut source: Option<Vec<rusqlite::types::Value>> = conn
        .prepare(&format!(
            "SELECT {COLONNES_HERITEES} FROM cards WHERE set_code = ?1 AND rarity = ?2 \
             ORDER BY rowid LIMIT 1"
        ))?
        .query_row((set_code, rarete), ligne_valeurs)
        .optional()?;
    if source.is_none() {
        source = conn
            .prepare(&format!(
                "SELECT {COLONNES_HERITEES} FROM cards WHERE set_code = ?1 \
                 ORDER BY rowid LIMIT 1"
            ))?
            .query_row((set_code,), ligne_valeurs)
            .optional()?;
    }
    let Some(source) = source else {
        return Ok(None);
    };

    // `rarete or s_rar` : le libellé imposé, ou celui de la source s'il est vide.
    let rarete_finale: rusqlite::types::Value = if rarete.is_empty() {
        source
            .get(2)
            .cloned()
            .unwrap_or(rusqlite::types::Value::Null)
    } else {
        rusqlite::types::Value::Text(rarete.to_owned())
    };

    let (colonne_ext, valeur_ext) = if a_extended_art {
        (", extended_art", ", ?18")
    } else {
        ("", "")
    };
    let sql = format!(
        "INSERT INTO cards \
           (card_uuid, card_image_uuid, set_code, rarity, rarity_code, \
            set_name, name, name_fr, \
            card_image_url, card_image_small, card_image_id, sort_order, \
            card_type, atk, def_val, level, attribute, race, \
            possessed, quantite, qualite, is_custom{colonne_ext}) \
         VALUES (?1, '', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, \
                 ?12, ?13, ?14, ?15, ?16, ?17, 0, 0, NULL, 0{valeur_ext})"
    );

    // L'ordre de `COLONNES_HERITEES`, moins `set_code` et `rarity` qui sont
    // repris à leur place, et `rarity` remplacée par le libellé imposé.
    let mut valeurs: Vec<rusqlite::types::Value> = Vec::with_capacity(18);
    let prendre = |i: usize| {
        source
            .get(i)
            .cloned()
            .unwrap_or(rusqlite::types::Value::Null)
    };
    valeurs.push(prendre(0)); // card_uuid
    valeurs.push(prendre(1)); // set_code — celui de la source, comme le Python
    valeurs.push(rarete_finale);
    for i in [3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16] {
        valeurs.push(prendre(i));
    }
    if a_extended_art {
        valeurs.push(rusqlite::types::Value::Integer(extended_art));
    }
    let refs: Vec<&dyn rusqlite::ToSql> =
        valeurs.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
    conn.execute(&sql, refs.as_slice())?;
    Ok(Some(conn.last_insert_rowid()))
}

/// Toutes les colonnes d'une ligne, sans les typer.
fn ligne_valeurs(ligne: &rusqlite::Row<'_>) -> rusqlite::Result<Vec<rusqlite::types::Value>> {
    (0..17).map(|i| ligne.get(i)).collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// La passe complète
// ─────────────────────────────────────────────────────────────────────────────

/// Ce qu'une passe a produit, ou pourquoi elle n'a rien produit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    /// La passe a tourné. `revid` est la révision de la Set list employée.
    Fait {
        /// Ce que SQLite a réellement modifié.
        bilan: Bilan,
        /// Révision de la page Yugipedia lue.
        revid: Option<i64>,
        /// Titre de la page employée.
        titre: String,
    },
    /// Le classeur est vide, ou n'a aucune ligne nommée.
    ClasseurVide,
    /// Aucune page « Set Card Lists » ne correspond au set.
    PageIntrouvable,
    /// La page existe mais ne décrit aucun tirage exploitable.
    StructureSterile {
        /// Titre de la page lue.
        titre: String,
    },
}

/// Exécute la passe artworks sur un classeur ouvert.
///
/// Portage de `completer_artworks_variantes`, réseau compris. L'enchaînement
/// est celui du Python — inventaire, structure, images, plan, écriture — mais
/// chacune des cinq étapes est une fonction publique et testable, ce que le
/// Python ne permet pas : là-bas les 370 lignes lisent, décident et écrivent
/// dans la même boucle.
///
/// # Ce que cette fonction ne fait pas
///
/// Elle **n'avale pas** les erreurs réseau. Le Python rend un bilan vide, si
/// bien qu'une passe empêchée et une passe sans effet se ressemblent. Ici la
/// distinction remonte, et c'est l'appelant qui choisit de continuer — comme
/// `create_classeur` le fait déjà, explicitement.
pub async fn passe(
    conn: &Connection,
    client: &ygo_sources::ClientHttp,
    langue: &str,
    raretes: &Priorites,
) -> Result<Issue> {
    use ygo_sources::yugipedia::{self, artwork, structure};

    let (inventaire, set_name) = lire_inventaire(conn)?;
    let (Some(premiere), false) = (inventaire.first(), set_name.is_empty()) else {
        return Ok(Issue::ClasseurVide);
    };
    // Le préfixe et la langue viennent d'un VRAI set_code du classeur, pas du
    // nom de dossier : ils peuvent différer (dossier « LOCR-JP » pour des codes
    // « LOCR-JP001 »), et c'est le set_code qui porte la langue.
    let code_reference = premiere.set_code.clone();
    let langue = if langue.is_empty() {
        artwork::langue_set_code(&code_reference)
    } else {
        langue.to_uppercase()
    };

    let titres = yugipedia::resoudre_pages(client, &set_name, None).await?;
    let Some(titre) = structure::choisir_page(&titres, &langue) else {
        return Ok(Issue::PageIntrouvable);
    };
    let titre = titre.to_owned();
    let (entrees, revid) = yugipedia::lire_set_list(client, &titre).await?;

    let slots = structure::slots(&entrees);
    if slots.is_empty() {
        return Ok(Issue::StructureSterile { titre });
    }
    let reference = structure::index_slots(&slots);
    let libelles = structure::libelles(&slots);

    // Les fichiers Yugipedia portent le nom de la FICHE, que la Set list
    // reprend ; la base peut en garder un autre (cf. `noms_set_list`). Les deux
    // sont cherchés : un fichier n'est rapproché d'une ligne que si son nom
    // correspond à l'un des deux.
    let selon_set_list = noms_set_list(&entrees);
    let mut noms: Vec<String> = inventaire.iter().map(|l| l.name.clone()).collect();
    for l in &inventaire {
        if let Some(n) = selon_set_list.get(&l.set_code) {
            if !noms.contains(n) {
                noms.push(n.clone());
            }
        }
    }
    let index = artwork::fichiers_pour_set(client, &code_reference, &noms, &langue).await?;
    let inventaire = rebaptiser(inventaire, &selon_set_list, &index);
    let prefixe = artwork::prefixe_set(&code_reference);

    let (fichiers, illustrations) = convertir(&index, &prefixe);
    let (actions, _) = planifier(
        &reference,
        &libelles,
        &inventaire,
        &fichiers,
        &artwork::abbr_rarete,
        &artwork::cle_comparaison,
        raretes,
    );
    let bilan = appliquer(conn, &actions, &illustrations)?;
    Ok(Issue::Fait {
        bilan,
        revid,
        titre,
    })
}

/// Le nom que la Set list donne à chaque numéro, numéro en majuscules.
fn noms_set_list(entrees: &[ygo_sources::yugipedia::EntreeSetList]) -> HashMap<String, String> {
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

/// Donne à une ligne le nom de la Set list quand c'est le seul que les
/// fichiers Yugipedia connaissent.
///
/// # Pourquoi — 2026-10-10
///
/// `LOCH-JP013` : la base dit « *Odd-Eyes Pendulum Dragon of the Four
/// Heavenly Dragons* », la fiche Yugipedia — et donc ses fichiers,
/// `OddEyesPendulumDragonFourHeavenlyDragons-LOCH-JP-UR.png` — l'autre nom.
/// Comparés au nom de la base, aucun fichier ne collait : les tirages en
/// variante gardaient l'image d'origine.
///
/// Le renommage n'existe que pour la passe — rien n'est écrit — et seulement
/// quand le nom de la base ne trouve **aucun** fichier et que celui de la Set
/// list en trouve : une carte déjà servie ne change jamais de clé.
fn rebaptiser(
    inventaire: Vec<LigneInventaire>,
    selon_set_list: &HashMap<String, String>,
    index: &ygo_sources::yugipedia::artwork::IndexFichiers,
) -> Vec<LigneInventaire> {
    use ygo_sources::yugipedia::artwork::cle_comparaison;
    let cles: HashSet<&String> = index.keys().map(|(cle, _)| cle).collect();
    inventaire
        .into_iter()
        .map(|mut l| {
            if !cles.contains(&cle_comparaison(&l.name)) {
                if let Some(nom) = selon_set_list.get(&l.set_code) {
                    if cles.contains(&cle_comparaison(nom)) {
                        l.name.clone_from(nom);
                    }
                }
            }
            l
        })
        .collect()
}

/// Traduit l'index de `ygo-sources` en ce que le planificateur consomme.
///
/// C'est la seule couture entre les deux crates, et elle est délibérément
/// étroite : le planificateur ne connaît ni `Candidat` ni `Artwork`, ce qui le
/// laisse testable sans rien de `ygo-sources`.
fn convertir(
    index: &ygo_sources::yugipedia::artwork::IndexFichiers,
    prefixe_set: &str,
) -> (IndexCandidats, IndexIllustrations) {
    use ygo_sources::yugipedia::artwork::formater_artwork;

    let mut fichiers: IndexCandidats = HashMap::new();
    let mut illustrations: IndexIllustrations = HashMap::new();
    for (cle, candidats) in index {
        for c in candidats {
            let art = formater_artwork(c, prefixe_set);
            illustrations.insert(
                c.infos.fichier.clone(),
                Illustration {
                    card_image_uuid: art.card_image_uuid,
                    card_image_id: art.card_image_id,
                    card_image_url: art.card_image_url,
                    card_image_small: art.card_image_small,
                },
            );
            fichiers
                .entry(cle.clone())
                .or_default()
                .push(FichierCandidat {
                    fichier: c.infos.fichier.clone(),
                    cle_carte: c.cle_carte.clone(),
                    rarete_abbr: c.infos.rarete_abbr.clone(),
                    variante: c.infos.variante.clone(),
                    card_url: c.card_url.clone(),
                    image_id: c.image_id,
                });
        }
    }
    (fichiers, illustrations)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn fichier(nom: &str, carte: &str, rarete: &str, variante: &str) -> FichierCandidat {
        FichierCandidat {
            fichier: nom.to_owned(),
            cle_carte: carte.to_owned(),
            rarete_abbr: rarete.to_owned(),
            variante: variante.to_owned(),
            card_url: format!("https://ms.yugipedia.com/{nom}"),
            image_id: -(nom.len() as i64),
        }
    }

    fn ligne(rowid: i64, numero: &str, rarete: &str, ext: i64) -> LigneInventaire {
        LigneInventaire {
            rowid,
            name: "Dark Magician".to_owned(),
            set_code: numero.to_owned(),
            rarity: rarete.to_owned(),
            ext,
        }
    }

    fn abbr(r: &str) -> String {
        match r {
            "Ultra Rare" => "UR".to_owned(),
            "Secret Rare" => "ScR".to_owned(),
            "Prismatic Secret Rare" => "PScR".to_owned(),
            _ => String::new(),
        }
    }

    fn cle(nom: &str) -> String {
        nom.to_lowercase()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect()
    }

    fn index(
        fichiers: Vec<(&str, &str, FichierCandidat)>,
    ) -> HashMap<(String, String), Vec<FichierCandidat>> {
        let mut m: HashMap<(String, String), Vec<FichierCandidat>> = HashMap::new();
        for (carte, rarete, f) in fichiers {
            m.entry((carte.to_owned(), rarete.to_owned()))
                .or_default()
                .push(f);
        }
        m
    }

    fn reference(entrees: Vec<(&str, &str, Vec<&str>)>) -> (Reference, HashMap<String, String>) {
        let mut r: Reference = BTreeMap::new();
        let mut libelles = HashMap::new();
        for (numero, rarete, variantes) in entrees {
            let cle_r = normaliser_rarete(rarete);
            libelles
                .entry(cle_r.clone())
                .or_insert_with(|| rarete.to_owned());
            r.insert(
                (numero.to_owned(), cle_r),
                variantes.into_iter().map(str::to_owned).collect(),
            );
        }
        (r, libelles)
    }

    fn plan(
        reference: &Reference,
        libelles: &HashMap<String, String>,
        inventaire: &[LigneInventaire],
        fichiers: &HashMap<(String, String), Vec<FichierCandidat>>,
    ) -> (Vec<Action>, Bilan) {
        planifier(
            reference,
            libelles,
            inventaire,
            fichiers,
            &abbr,
            &cle,
            &options(),
        )
    }

    /// La liste des Options des tests unitaires.
    ///
    /// Les fixtures de ce module emploient déjà des libellés canoniques ; la
    /// canonisation y est l'identité, et c'est voulu — ces tests éprouvent
    /// l'appariement des fichiers, pas l'orthographe des raretés. Le test
    /// [`tests::une_rarete_abregee_ne_cree_plus_de_doublon`] s'occupe du reste.
    fn options() -> Priorites {
        Priorites::depuis_paires([
            ("Common", 1),
            ("Super Rare", 3),
            ("Ultra Rare", 4),
            ("Secret Rare", 5),
            ("Platinum Secret Rare", 7),
            ("Collector's Rare", 9),
            ("Ultimate Rare", 11),
            ("Quarter Century Secret Rare", 12),
            ("Prismatic Secret Rare", 10),
        ])
    }

    // ── La cause des doublons, et sa correction ─────────────────────────────

    /// Le défaut mesuré sur les bases réelles, réduit à trois lignes.
    ///
    /// Yugipedia écrit `PlScR` sur `RA02`, le classeur porte
    /// `Platinum Secret Rare` : c'est le **même tirage**. Avec la seule clé du
    /// Python (`plscr` ≠ `platinumsecretrare`), la passe ne le retrouve pas,
    /// conclut qu'il manque, et l'insère — 567 fois sur `RA02`.
    ///
    /// Avec la clé canonisée, elle le retrouve et n'insère rien.
    /// `LOCH-JP013` : le nom de la base ne trouve aucun fichier, celui de la
    /// Set list oui — la ligne prend le second, pour la passe seulement. Une
    /// ligne que son propre nom sert garde le sien.
    #[test]
    fn une_ligne_prend_le_nom_de_la_set_list_quand_seul_lui_a_des_fichiers() {
        let candidat = |cle: &str| ygo_sources::yugipedia::artwork::Candidat {
            segment: cle.to_owned(),
            cle_carte: cle.to_owned(),
            infos: ygo_sources::yugipedia::artwork::InfosFichier {
                fichier: format!("{cle}.png"),
                langue: "JP".to_owned(),
                rarete_abbr: "UR".to_owned(),
                edition: String::new(),
                variante: "EA".to_owned(),
            },
            card_url: String::new(),
            image_id: -1,
            uuid: String::new(),
        };
        let mut index = ygo_sources::yugipedia::artwork::IndexFichiers::new();
        for cle in [
            "oddeyespendulumdragonfourheavenlydragons",
            "gagagagirlcellphonesubtraction",
        ] {
            index.insert((cle.to_owned(), "UR".to_owned()), vec![candidat(cle)]);
        }
        let selon_set_list = HashMap::from([
            (
                "LOCH-JP013".to_owned(),
                "Odd-Eyes Pendulum Dragon, Four Heavenly Dragons".to_owned(),
            ),
            ("LOCH-JP012".to_owned(), "Autre nom".to_owned()),
        ]);
        let mut oe = ligne(1, "LOCH-JP013", "Ultra Rare", 1);
        oe.name = "Odd-Eyes Pendulum Dragon of the Four Heavenly Dragons".to_owned();
        let mut gg = ligne(2, "LOCH-JP012", "Ultra Rare", 1);
        gg.name = "Gagaga Girl - Cell Phone Subtraction".to_owned();
        let r = rebaptiser(vec![oe, gg], &selon_set_list, &index);
        assert_eq!(r[0].name, "Odd-Eyes Pendulum Dragon, Four Heavenly Dragons");
        assert_eq!(
            r[1].name, "Gagaga Girl - Cell Phone Subtraction",
            "servie par son nom : gardé"
        );
    }

    #[test]
    fn une_rarete_abregee_ne_cree_plus_de_doublon() {
        let (r, lib) = reference(vec![("RA02-EN001", "PlScR", vec![""])]);
        let inv = vec![ligne(1, "RA02-EN001", "Platinum Secret Rare", 0)];
        let f = index(vec![]);

        let (actions, bilan) = plan(&r, &lib, &inv, &f);
        assert_eq!(
            bilan.ajoutes, 0,
            "le tirage existe déjà, sous l'autre écriture"
        );
        assert!(
            !actions.iter().any(|a| matches!(a, Action::Inserer { .. })),
            "aucune insertion"
        );
    }

    /// Les sept écritures de `RA02`, d'un coup : la Set list en abrégé, le
    /// classeur en toutes lettres, et pas une ligne créée.
    #[test]
    fn les_sept_raretes_de_ra02_s_apparient_toutes() {
        let couples = [
            ("CR", "Collector's Rare"),
            ("PlScR", "Platinum Secret Rare"),
            ("QCScR", "Quarter Century Secret Rare"),
            ("ScR", "Secret Rare"),
            ("SR", "Super Rare"),
            ("UR", "Ultra Rare"),
            ("UtR", "Ultimate Rare"),
        ];
        let (r, lib) = reference(
            couples
                .iter()
                .map(|(abrege, _)| ("RA02-EN001", *abrege, vec![""]))
                .collect(),
        );
        let inv: Vec<LigneInventaire> = couples
            .iter()
            .enumerate()
            .map(|(i, (_, complet))| ligne(i as i64 + 1, "RA02-EN001", complet, 0))
            .collect();

        let (actions, bilan) = plan(&r, &lib, &inv, &index(vec![]));
        assert_eq!(bilan.ajoutes, 0);
        assert!(actions.is_empty());
    }

    /// La coquille de `RA05` se rapproche elle aussi : `PLatinum Secret Rare`
    /// dans le classeur, `Platinum Secret Rare` sur le wiki.
    ///
    /// Ce cas-là, le Python le traitait déjà — sa clé minuscule les rapproche.
    /// Ce qu'il ne faisait pas, c'est **propager la bonne orthographe** : une
    /// ligne créée à côté héritait de la coquille, et l'étendait. La seconde
    /// moitié du test est donc la partie neuve.
    #[test]
    fn la_coquille_de_ra05_ne_se_propage_pas_aux_lignes_creees() {
        let (r, lib) = reference(vec![("RA05-EN136", "Platinum Secret Rare", vec!["", "AA"])]);
        let inv = vec![ligne(1, "RA05-EN136", "PLatinum Secret Rare", 0)];

        let (actions, bilan) = plan(&r, &lib, &inv, &index(vec![]));
        assert_eq!(bilan.ajoutes, 1, "seule la variante AA manque");
        assert!(
            matches!(
                actions.first(),
                Some(Action::Inserer { rarete, .. }) if rarete == "Platinum Secret Rare"
            ),
            "la ligne créée porte la forme des Options, pas la coquille locale : {:?}",
            actions.first()
        );
    }

    /// Le garde-fou : deux raretés **différentes** doivent rester différentes.
    /// Un tirage `Secret Rare` que le classeur n'a pas doit toujours être créé.
    #[test]
    fn deux_raretes_distinctes_restent_distinctes() {
        let (r, lib) = reference(vec![("RA02-EN001", "ScR", vec![""])]);
        let inv = vec![ligne(1, "RA02-EN001", "Ultra Rare", 0)];

        let (actions, bilan) = plan(&r, &lib, &inv, &index(vec![]));
        assert_eq!(bilan.ajoutes, 1, "le Secret Rare manque vraiment");
        assert!(matches!(
            actions.first(),
            Some(Action::Inserer { rarete, .. }) if rarete == "Secret Rare"
        ));
    }

    /// Et la ligne créée porte le libellé **canonique**, pas l'abréviation du
    /// wiki : c'est ce qui empêche la passe de réintroduire le problème
    /// qu'elle vient de cesser de causer.
    #[test]
    fn une_ligne_creee_porte_le_libelle_canonique() {
        let (r, lib) = reference(vec![("RA02-EN001", "PlScR", vec!["", "AA"])]);
        let inv = vec![ligne(1, "RA02-EN001", "Platinum Secret Rare", 0)];

        let (actions, bilan) = plan(&r, &lib, &inv, &index(vec![]));
        assert_eq!(bilan.ajoutes, 1, "la variante AA manque");
        assert!(matches!(
            actions.first(),
            Some(Action::Inserer { rarete, .. }) if rarete == "Platinum Secret Rare"
        ));
    }

    // ── Le piège central : un fichier sans suffixe ──────────────────────────

    #[test]
    fn un_fichier_nu_illustre_une_variante_quand_le_tirage_n_existe_qu_en_variante() {
        // Cas `RA05-EN083` : Dark Magician *est* du stamp artwork, et ses
        // fichiers n'ont aucun suffixe — Yugipedia n'en ajoute que pour
        // désambiguïser. Refuser le fichier nu ici, c'est manquer la variante.
        let (r, lib) = reference(vec![("RA05-EN083", "Ultra Rare", vec!["AA"])]);
        let inv = vec![ligne(1, "RA05-EN083", "Ultra Rare", 0)];
        let f = index(vec![(
            "darkmagician",
            "UR",
            fichier("DarkMagician-RA05-EN-UR-1E.png", "darkmagician", "UR", ""),
        )]);

        let (actions, bilan) = plan(&r, &lib, &inv, &f);
        assert_eq!(bilan.illustrations, 1);
        assert!(matches!(
            actions.first(),
            Some(Action::PoserIllustration {
                cible: Cible::Existante(1),
                ..
            })
        ));
    }

    #[test]
    fn un_fichier_nu_est_refuse_quand_le_meme_couple_a_un_tirage_normal() {
        // Le danger : ce fichier nu appartient à la ligne normale. Le poser sur
        // la ligne variante afficherait DEUX FOIS la même illustration.
        let (r, lib) = reference(vec![("LOCR-JP001", "Ultra Rare", vec!["", "EA"])]);
        let inv = vec![
            ligne(1, "LOCR-JP001", "Ultra Rare", 0),
            ligne(2, "LOCR-JP001", "Ultra Rare", 1),
        ];
        let f = index(vec![(
            "darkmagician",
            "UR",
            fichier("DarkMagician-LOCR-JP-UR.png", "darkmagician", "UR", ""),
        )]);

        let (actions, bilan) = plan(&r, &lib, &inv, &f);
        assert_eq!(bilan.illustrations, 0, "aucune image posée");
        assert!(actions.is_empty(), "et aucune action du tout");
    }

    #[test]
    fn un_fichier_suffixe_passe_meme_avec_un_tirage_normal() {
        // Avec un suffixe, plus d'ambiguïté : le fichier est bien la variante.
        let (r, lib) = reference(vec![("LOCR-JP001", "Ultra Rare", vec!["", "EA"])]);
        let inv = vec![
            ligne(1, "LOCR-JP001", "Ultra Rare", 0),
            ligne(2, "LOCR-JP001", "Ultra Rare", 1),
        ];
        let f = index(vec![(
            "darkmagician",
            "UR",
            fichier("DarkMagician-LOCR-JP-UR-EA.png", "darkmagician", "UR", "EA"),
        )]);

        let (actions, bilan) = plan(&r, &lib, &inv, &f);
        assert_eq!(bilan.illustrations, 1);
        // Et c'est bien la ligne Overframe, la seconde, qui la reçoit.
        assert!(matches!(
            actions.first(),
            Some(Action::PoserIllustration {
                cible: Cible::Existante(2),
                ..
            })
        ));
    }

    // ── Ordre de préférence ─────────────────────────────────────────────────

    #[test]
    fn la_famille_du_slot_passe_avant_tout_autre_suffixe() {
        let candidats = vec![
            fichier("a-AA.png", "c", "UR", "AA"),
            fichier("b-EA.png", "c", "UR", "EA"),
            fichier("c.png", "c", "UR", ""),
        ];
        let vide = HashSet::new();
        assert_eq!(
            choisir_fichier(&candidats, "EA", &vide, true).map(|c| c.fichier.as_str()),
            Some("b-EA.png")
        );
        assert_eq!(
            choisir_fichier(&candidats, "AA", &vide, true).map(|c| c.fichier.as_str()),
            Some("a-AA.png")
        );
    }

    #[test]
    fn a_defaut_de_la_bonne_famille_n_importe_quel_suffixe() {
        let candidats = vec![
            fichier("nu.png", "c", "UR", ""),
            fichier("aa-AA.png", "c", "UR", "AA"),
        ];
        let vide = HashSet::new();
        assert_eq!(
            choisir_fichier(&candidats, "EA", &vide, true).map(|c| c.fichier.as_str()),
            Some("aa-AA.png"),
            "un suffixé, même d'une autre famille, passe avant un nu"
        );
    }

    #[test]
    fn un_fichier_ne_sert_jamais_deux_fois() {
        let candidats = vec![fichier("seul-EA.png", "c", "UR", "EA")];
        let mut utilises = HashSet::new();
        assert!(choisir_fichier(&candidats, "EA", &utilises, true).is_some());
        utilises.insert("seul-EA.png".to_owned());
        assert!(
            choisir_fichier(&candidats, "EA", &utilises, true).is_none(),
            "épuisé, on ne rend rien plutôt que de doublonner"
        );
    }

    #[test]
    fn deux_slots_de_variante_recoivent_deux_fichiers_differents() {
        let (r, lib) = reference(vec![("A-EN001", "Ultra Rare", vec!["AA", "EA"])]);
        let inv = vec![
            ligne(1, "A-EN001", "Ultra Rare", 0),
            ligne(2, "A-EN001", "Ultra Rare", 1),
        ];
        let f = index(vec![
            (
                "darkmagician",
                "UR",
                fichier("un-AA.png", "darkmagician", "UR", "AA"),
            ),
            (
                "darkmagician",
                "UR",
                fichier("deux-EA.png", "darkmagician", "UR", "EA"),
            ),
        ]);

        let (actions, bilan) = plan(&r, &lib, &inv, &f);
        assert_eq!(bilan.illustrations, 2);
        let fichiers: Vec<&str> = actions
            .iter()
            .filter_map(|a| match a {
                Action::PoserIllustration { fichier, .. } => Some(fichier.fichier.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(fichiers, ["un-AA.png", "deux-EA.png"]);
    }

    // ── Repli sur une autre rareté ──────────────────────────────────────────

    #[test]
    fn le_repli_n_accepte_un_nu_que_si_le_numero_est_tout_variante() {
        // Le numéro n'existe qu'en variante : aucun fichier de la carte ne
        // peut être l'illustration d'origine, donc emprunter un nu est sûr.
        // Le fichier est rangé sous une rareté que la référence ne vise pas :
        // `candidats` est vide, seul le repli peut le trouver.
        let (r, lib) = reference(vec![("A-EN001", "Ultra Rare", vec!["AA"])]);
        let inv = vec![ligne(1, "A-EN001", "Ultra Rare", 0)];
        let f = index(vec![(
            "darkmagician",
            "SCR",
            fichier("nu.png", "darkmagician", "ScR", ""),
        )]);

        let (_, bilan) = plan(&r, &lib, &inv, &f);
        assert_eq!(bilan.illustrations, 1);
        assert_eq!(bilan.reutilisees, 1, "empruntée à une autre rareté");
    }

    #[test]
    fn le_repli_refuse_ce_meme_nu_si_le_numero_a_un_tirage_normal() {
        // Même montage que ci-dessus, à un slot près : un second couple du
        // MÊME numéro a un tirage normal. `tout_variante` tombe, et le repli
        // refuse le nu — c'est ce seul bit qui sépare les deux tests.
        let (r, lib) = reference(vec![
            ("A-EN001", "Ultra Rare", vec!["AA"]),
            ("A-EN001", "Prismatic Secret Rare", vec![""]),
        ]);
        let inv = vec![
            ligne(1, "A-EN001", "Ultra Rare", 0),
            ligne(2, "A-EN001", "Prismatic Secret Rare", 0),
        ];
        let f = index(vec![(
            "darkmagician",
            "SCR",
            fichier("nu.png", "darkmagician", "ScR", ""),
        )]);

        let (_, bilan) = plan(&r, &lib, &inv, &f);
        assert_eq!(bilan.illustrations, 0);
        assert_eq!(bilan.reutilisees, 0);
    }

    #[test]
    fn la_retenue_des_fichiers_ne_traverse_pas_les_raretes() {
        // Comportement du Python reproduit tel quel : `utilises` est remis à
        // zéro à chaque couple (numéro, rareté). Deux raretés d'un même numéro
        // peuvent donc recevoir LE MÊME fichier — ce qui est voulu, deux
        // raretés d'une même variante montrant la même illustration.
        let (r, lib) = reference(vec![
            ("A-EN001", "Ultra Rare", vec!["AA"]),
            ("A-EN001", "Secret Rare", vec!["AA"]),
        ]);
        let inv = vec![
            ligne(1, "A-EN001", "Ultra Rare", 0),
            ligne(2, "A-EN001", "Secret Rare", 0),
        ];
        let f = index(vec![(
            "darkmagician",
            "PSCR",
            fichier("nu.png", "darkmagician", "PScR", ""),
        )]);

        let (actions, bilan) = plan(&r, &lib, &inv, &f);
        assert_eq!(bilan.illustrations, 2, "un fichier, deux lignes");
        assert_eq!(bilan.reutilisees, 2);
        let poses: Vec<(i64, &str)> = actions
            .iter()
            .filter_map(|a| match a {
                Action::PoserIllustration {
                    cible: Cible::Existante(rowid),
                    fichier,
                    ..
                } => Some((*rowid, fichier.fichier.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(
            poses,
            [(2, "nu.png"), (1, "nu.png")],
            "ScR avant UR (tri des clés)"
        );
    }

    #[test]
    fn un_couple_sans_ligne_au_classeur_fait_naitre_ses_tirages() {
        // Le garde-fou porte sur le NUMÉRO, pas sur le couple : dès que le
        // numéro existe quelque part au classeur, une rareté que le classeur
        // ignore se voit créer ses lignes — et illustrer.
        let (r, lib) = reference(vec![
            ("A-EN001", "Ultra Rare", vec!["AA"]),
            ("A-EN001", "Secret Rare", vec!["AA"]),
        ]);
        let inv = vec![ligne(1, "A-EN001", "Ultra Rare", 0)];
        let f = index(vec![(
            "darkmagician",
            "SCR",
            fichier("nu.png", "darkmagician", "ScR", ""),
        )]);

        let (actions, bilan) = plan(&r, &lib, &inv, &f);
        assert_eq!(bilan.ajoutes, 1, "la ligne Secret Rare manquait");
        assert_eq!(bilan.absents, 0, "le numéro, lui, est bien au classeur");
        // La ligne créée porte le libellé de la RÉFÉRENCE, faute d'orthographe
        // au classeur pour cette rareté.
        assert!(matches!(
            actions.first(),
            Some(Action::Inserer { rarete, indice: 0, .. }) if rarete == "Secret Rare"
        ));
        // Et elle est illustrée comme n'importe quelle autre.
        assert!(matches!(
            actions.get(1),
            Some(Action::PoserIllustration {
                cible: Cible::Nouvelle(0),
                ..
            })
        ));
    }

    #[test]
    fn le_repli_refuse_un_nu_quand_le_numero_a_un_tirage_normal_ailleurs() {
        // Un seul couple a un tirage normal, et cela suffit à interdire les
        // fichiers nus empruntés pour TOUT le numéro.
        let (r, lib) = reference(vec![
            ("A-EN001", "Ultra Rare", vec!["AA"]),
            ("A-EN001", "Secret Rare", vec![""]),
        ]);
        let inv = vec![
            ligne(1, "A-EN001", "Ultra Rare", 0),
            ligne(2, "A-EN001", "Secret Rare", 0),
        ];
        let f = index(vec![(
            "darkmagician",
            "SCR",
            fichier("nu.png", "darkmagician", "ScR", ""),
        )]);

        let (_, bilan) = plan(&r, &lib, &inv, &f);
        assert_eq!(bilan.illustrations, 0);
        assert_eq!(bilan.reutilisees, 0);
    }

    // ── Complétion et drapeaux ──────────────────────────────────────────────

    #[test]
    fn les_lignes_manquantes_sont_creees_jamais_au_dela() {
        let (r, lib) = reference(vec![("A-EN001", "Ultra Rare", vec!["", "AA", "EA"])]);
        let inv = vec![ligne(1, "A-EN001", "Ultra Rare", 0)];
        let (actions, bilan) = plan(&r, &lib, &inv, &HashMap::new());

        assert_eq!(bilan.ajoutes, 2, "une ligne présente, trois attendues");
        let inserees: Vec<&Action> = actions
            .iter()
            .filter(|a| matches!(a, Action::Inserer { .. }))
            .collect();
        assert_eq!(inserees.len(), 2);
        // La troisième est extended art : elle naît avec son drapeau.
        assert!(matches!(
            inserees[1],
            Action::Inserer {
                extended_art: 1,
                ..
            }
        ));
        assert!(matches!(
            inserees[0],
            Action::Inserer {
                extended_art: 0,
                ..
            }
        ));
    }

    #[test]
    fn le_drapeau_monte_mais_ne_descend_jamais() {
        // Slot extended art sur une ligne qui ne l'est pas : on monte.
        let (r, lib) = reference(vec![("A-EN001", "Ultra Rare", vec!["EA"])]);
        let inv = vec![ligne(1, "A-EN001", "Ultra Rare", 0)];
        let (actions, bilan) = plan(&r, &lib, &inv, &HashMap::new());
        assert_eq!(bilan.flags, 1);
        assert!(matches!(
            actions.first(),
            Some(Action::Flaguer { rowid: 1 })
        ));

        // Slot normal sur une ligne Overframe : on ne touche à rien.
        let (r, lib) = reference(vec![("A-EN001", "Ultra Rare", vec![""])]);
        let inv = vec![ligne(1, "A-EN001", "Ultra Rare", 1)];
        let (actions, bilan) = plan(&r, &lib, &inv, &HashMap::new());
        assert_eq!(bilan.flags, 0);
        assert!(actions.is_empty(), "jamais de déclassement");
    }

    #[test]
    fn les_lignes_sont_appariees_cadre_normal_d_abord() {
        // La ligne Overframe a le plus petit rowid : elle doit quand même
        // passer en second, parce que le tri est (extended_art, rowid).
        let (r, lib) = reference(vec![("A-EN001", "Ultra Rare", vec!["", "EA"])]);
        let inv = vec![
            ligne(1, "A-EN001", "Ultra Rare", 1),
            ligne(2, "A-EN001", "Ultra Rare", 0),
        ];
        let f = index(vec![(
            "darkmagician",
            "UR",
            fichier("ea.png", "darkmagician", "UR", "EA"),
        )]);
        let (actions, _) = plan(&r, &lib, &inv, &f);
        // Le slot EA est le second : il vise donc la ligne de rowid 1.
        assert!(matches!(
            actions.first(),
            Some(Action::PoserIllustration {
                cible: Cible::Existante(1),
                ..
            })
        ));
    }

    #[test]
    fn un_numero_absent_du_classeur_est_compte_et_non_cree() {
        let (r, lib) = reference(vec![
            ("A-EN001", "Ultra Rare", vec![""]),
            ("A-EN999", "Ultra Rare", vec!["AA"]),
        ]);
        let inv = vec![ligne(1, "A-EN001", "Ultra Rare", 0)];
        let (actions, bilan) = plan(&r, &lib, &inv, &HashMap::new());
        assert_eq!(bilan.absents, 1);
        assert_eq!(bilan.ajoutes, 0, "on ne fabrique pas un numéro inconnu");
        assert!(actions.is_empty());
    }

    #[test]
    fn la_rarete_ecrite_est_l_orthographe_du_classeur() {
        // Le wiki écrit « PLatinum Secret Rare » ; le classeur écrit
        // « Prismatic Secret Rare ». La ligne créée doit prendre celle du
        // classeur, sans quoi elle ne se raccrocherait à rien.
        let (r, lib) = reference(vec![("A-EN001", "prismatic secret rare", vec!["", "AA"])]);
        let inv = vec![ligne(1, "A-EN001", "Prismatic Secret Rare", 0)];
        let (actions, _) = plan(&r, &lib, &inv, &HashMap::new());
        let Some(Action::Inserer { rarete, .. }) = actions.first() else {
            unreachable!("une ligne doit être créée")
        };
        assert_eq!(rarete, "Prismatic Secret Rare");
    }

    #[test]
    fn un_inventaire_vide_ne_produit_rien() {
        let (r, lib) = reference(vec![("A-EN001", "Ultra Rare", vec!["AA"])]);
        let (actions, bilan) = plan(&r, &lib, &[], &HashMap::new());
        assert!(actions.is_empty());
        assert_eq!(
            bilan,
            Bilan {
                absents: 1,
                ..Bilan::default()
            }
        );
    }

    // ── L'applicateur ───────────────────────────────────────────────────────

    /// Un classeur jouable, au schéma réel, peuplé de lignes minimales.
    fn base(lignes: &[(i64, &str, &str, &str, i64)]) -> Connection {
        let conn = ygo_db::connexion::en_memoire().unwrap();
        ygo_db::schema::creer_tout(&conn, ygo_db::schema::DDL_CLASSEUR).unwrap();
        for (rowid, nom, numero, rarete, ext) in lignes {
            conn.execute(
                "INSERT INTO cards (rowid, card_uuid, name, name_fr, set_code, rarity, \
                     rarity_code, set_name, card_image_url, card_image_small, \
                     card_image_id, sort_order, card_type, atk, def_val, level, \
                     attribute, race, possessed, quantite, extended_art) \
                 VALUES (?1, 'uuid-source', ?2, 'Magicien Sombre', ?3, ?4, '(UR)', \
                     'Le Set', 'http://img/x.jpg', 'http://img/x-s.jpg', 42, 7, \
                     'Monster', 2500, 2100, 7, 'DARK', 'Spellcaster', 1, 3, ?5)",
                rusqlite::params![rowid, nom, numero, rarete, ext],
            )
            .unwrap();
        }
        conn
    }

    fn illustration() -> HashMap<String, Illustration> {
        HashMap::from([(
            "nouvelle-AA.png".to_owned(),
            Illustration {
                card_image_uuid: "yugipedia:nouvelle-AA.png".to_owned(),
                card_image_id: -7,
                card_image_url: "http://ms/nouvelle-AA.png".to_owned(),
                card_image_small: "http://ms/nouvelle-AA.png".to_owned(),
            },
        )])
    }

    fn ligne_creee(
        conn: &Connection,
        rowid: i64,
    ) -> (String, String, i64, i64, i64, Option<String>) {
        conn.query_row(
            "SELECT rarity, name_fr, atk, possessed, extended_art, qualite \
             FROM cards WHERE rowid = ?1",
            [rowid],
            |l| {
                Ok((
                    l.get(0)?,
                    l.get(1)?,
                    l.get(2)?,
                    l.get(3)?,
                    l.get(4)?,
                    l.get(5)?,
                ))
            },
        )
        .unwrap()
    }

    #[test]
    fn une_ligne_creee_herite_des_metadonnees_et_nait_non_possedee() {
        let conn = base(&[(1, "Dark Magician", "A-EN001", "Ultra Rare", 0)]);
        let actions = vec![Action::Inserer {
            indice: 0,
            numero: "A-EN001".to_owned(),
            rarete: "Secret Rare".to_owned(),
            extended_art: 1,
        }];
        let bilan = appliquer(&conn, &actions, &HashMap::new()).unwrap();
        assert_eq!(bilan.ajoutes, 1);

        let (rarete, nom_fr, atk, possede, ext, qualite) = ligne_creee(&conn, 2);
        assert_eq!(rarete, "Secret Rare", "la rareté est imposée");
        assert_eq!(ext, 1, "le drapeau aussi");
        assert_eq!(nom_fr, "Magicien Sombre", "le reste est hérité");
        assert_eq!(atk, 2500);
        assert_eq!(possede, 0, "une ligne créée n'est jamais possédée");
        assert_eq!(qualite, None);
    }

    #[test]
    fn la_ligne_source_est_celle_de_la_meme_rarete_quand_elle_existe() {
        // Deux lignes du même numéro, des rangs de tri différents : la source
        // doit être celle de la rareté visée, pas la première venue.
        let conn = base(&[
            (1, "Dark Magician", "A-EN001", "Ultra Rare", 0),
            (2, "Dark Magician", "A-EN001", "Secret Rare", 0),
        ]);
        conn.execute("UPDATE cards SET sort_order = 99 WHERE rowid = 2", [])
            .unwrap();

        let actions = vec![Action::Inserer {
            indice: 0,
            numero: "A-EN001".to_owned(),
            rarete: "Secret Rare".to_owned(),
            extended_art: 0,
        }];
        appliquer(&conn, &actions, &HashMap::new()).unwrap();
        let rang: i64 = conn
            .query_row("SELECT sort_order FROM cards WHERE rowid = 3", [], |l| {
                l.get(0)
            })
            .unwrap();
        assert_eq!(rang, 99, "héritée de la ligne Secret Rare");
    }

    #[test]
    fn sans_ligne_du_numero_rien_ne_nait_et_l_illustration_ne_se_perd_pas() {
        // Le piège : le plan vise la ligne créée par son INDICE. Si la création
        // échoue, l'indice ne doit surtout pas glisser sur une autre ligne.
        let conn = base(&[(1, "Dark Magician", "A-EN001", "Ultra Rare", 0)]);
        let actions = vec![
            Action::Inserer {
                indice: 0,
                numero: "INCONNU-001".to_owned(),
                rarete: "Ultra Rare".to_owned(),
                extended_art: 0,
            },
            Action::Inserer {
                indice: 1,
                numero: "A-EN001".to_owned(),
                rarete: "Ultra Rare".to_owned(),
                extended_art: 0,
            },
            Action::PoserIllustration {
                cible: Cible::Nouvelle(0),
                fichier: fichier("nouvelle-AA.png", "darkmagician", "UR", "AA"),
                extended_art: None,
            },
            Action::PoserIllustration {
                cible: Cible::Nouvelle(1),
                fichier: fichier("nouvelle-AA.png", "darkmagician", "UR", "AA"),
                extended_art: None,
            },
        ];
        let bilan = appliquer(&conn, &actions, &illustration()).unwrap();
        assert_eq!(bilan.ajoutes, 1, "seule la seconde a une source");
        assert_eq!(bilan.illustrations, 1, "et seule elle est illustrée");

        let uuid: Option<String> = conn
            .query_row(
                "SELECT card_image_uuid FROM cards WHERE rowid = 2",
                [],
                |l| l.get(0),
            )
            .unwrap();
        assert_eq!(uuid.as_deref(), Some("yugipedia:nouvelle-AA.png"));
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM cards", [], |l| l.get(0))
            .unwrap();
        assert_eq!(total, 2, "aucune ligne fantôme");
    }

    #[test]
    fn la_garde_d_idempotence_epargne_une_ligne_deja_servie() {
        let conn = base(&[(1, "Dark Magician", "A-EN001", "Ultra Rare", 0)]);
        conn.execute(
            "UPDATE cards SET card_image_uuid = 'yugipedia:ancienne.png' WHERE rowid = 1",
            [],
        )
        .unwrap();
        let actions = vec![Action::PoserIllustration {
            cible: Cible::Existante(1),
            fichier: fichier("nouvelle-AA.png", "darkmagician", "UR", "AA"),
            extended_art: None,
        }];
        let bilan = appliquer(&conn, &actions, &illustration()).unwrap();
        assert_eq!(bilan.illustrations, 0, "déjà servie");
        let uuid: String = conn
            .query_row(
                "SELECT card_image_uuid FROM cards WHERE rowid = 1",
                [],
                |l| l.get(0),
            )
            .unwrap();
        assert_eq!(uuid, "yugipedia:ancienne.png", "et intacte");
    }

    #[test]
    fn une_illustration_absente_de_l_index_ne_touche_pas_la_ligne() {
        let conn = base(&[(1, "Dark Magician", "A-EN001", "Ultra Rare", 0)]);
        let actions = vec![Action::PoserIllustration {
            cible: Cible::Existante(1),
            fichier: fichier("jamais-vue.png", "darkmagician", "UR", "AA"),
            extended_art: Some(1),
        }];
        let bilan = appliquer(&conn, &actions, &illustration()).unwrap();
        assert_eq!(bilan.illustrations, 0);
        let ext: i64 = conn
            .query_row("SELECT extended_art FROM cards WHERE rowid = 1", [], |l| {
                l.get(0)
            })
            .unwrap();
        assert_eq!(ext, 0, "le drapeau n'a pas bougé non plus");
    }

    #[test]
    fn poser_une_illustration_peut_monter_le_drapeau_dans_le_meme_ordre() {
        let conn = base(&[(1, "Dark Magician", "A-EN001", "Ultra Rare", 0)]);
        let actions = vec![Action::PoserIllustration {
            cible: Cible::Existante(1),
            fichier: fichier("nouvelle-AA.png", "darkmagician", "UR", "AA"),
            extended_art: Some(1),
        }];
        appliquer(&conn, &actions, &illustration()).unwrap();
        let (id, ext): (i64, i64) = conn
            .query_row(
                "SELECT card_image_id, extended_art FROM cards WHERE rowid = 1",
                [],
                |l| Ok((l.get(0)?, l.get(1)?)),
            )
            .unwrap();
        assert_eq!(id, -7);
        assert_eq!(ext, 1);
    }

    #[test]
    fn le_drapeau_ne_remonte_pas_une_ligne_deja_marquee() {
        let conn = base(&[(1, "Dark Magician", "A-EN001", "Ultra Rare", 1)]);
        let bilan = appliquer(&conn, &[Action::Flaguer { rowid: 1 }], &HashMap::new()).unwrap();
        assert_eq!(bilan.flags, 0, "la garde COALESCE(extended_art,0)=0 tient");
    }

    // ── La lecture de l'inventaire ──────────────────────────────────────────

    #[test]
    fn l_inventaire_ecarte_les_lignes_sans_nom_et_majuscule_les_numeros() {
        let conn = base(&[(1, "Dark Magician", "a-en001", "Ultra Rare", 0)]);
        conn.execute(
            "INSERT INTO cards (rowid, name, set_code, rarity) VALUES (2, '', 'A-EN002', 'UR')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cards (rowid, name, set_code, rarity) VALUES (3, NULL, 'A-EN003', 'UR')",
            [],
        )
        .unwrap();

        let (lignes, set_name) = lire_inventaire(&conn).unwrap();
        assert_eq!(lignes.len(), 1, "deux résidus écartés");
        assert_eq!(lignes[0].set_code, "A-EN001", "numéro en majuscules");
        assert_eq!(set_name, "Le Set");
    }

    #[test]
    fn le_set_name_est_le_premier_non_vide() {
        let conn = base(&[
            (1, "Dark Magician", "A-EN001", "Ultra Rare", 0),
            (2, "Dark Magician", "A-EN002", "Ultra Rare", 0),
        ]);
        conn.execute("UPDATE cards SET set_name = '' WHERE rowid = 1", [])
            .unwrap();
        let (_, set_name) = lire_inventaire(&conn).unwrap();
        assert_eq!(set_name, "Le Set", "la première ligne ne le porte pas");
    }

    #[test]
    fn un_classeur_sans_colonne_extended_art_ne_fait_pas_echouer_la_passe() {
        // Les classeurs de génération 1 n'ont pas la colonne. Le Python s'en
        // passe silencieusement plutôt que de lever.
        let conn = ygo_db::connexion::en_memoire().unwrap();
        conn.execute(
            "CREATE TABLE cards (card_uuid TEXT, card_image_uuid TEXT, card_image_id INTEGER, \
                 set_code TEXT, rarity TEXT, rarity_code TEXT, set_name TEXT, name TEXT, \
                 name_fr TEXT, card_image_url TEXT, card_image_small TEXT, sort_order INTEGER, \
                 card_type TEXT, atk INTEGER, def_val INTEGER, level INTEGER, attribute TEXT, \
                 race TEXT, possessed INTEGER, quantite INTEGER, qualite TEXT, is_custom INTEGER)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cards (rowid, name, set_code, rarity) \
             VALUES (1, 'Dark Magician', 'A-EN001', 'Ultra Rare')",
            [],
        )
        .unwrap();

        let (lignes, _) = lire_inventaire(&conn).unwrap();
        assert_eq!(lignes.len(), 1);
        assert_eq!(lignes[0].ext, 0, "faute de colonne, tout est normal");

        let actions = vec![
            Action::Flaguer { rowid: 1 },
            Action::Inserer {
                indice: 0,
                numero: "A-EN001".to_owned(),
                rarete: "Secret Rare".to_owned(),
                extended_art: 1,
            },
            Action::PoserIllustration {
                cible: Cible::Nouvelle(0),
                fichier: fichier("nouvelle-AA.png", "darkmagician", "UR", "AA"),
                extended_art: Some(1),
            },
        ];
        let bilan = appliquer(&conn, &actions, &illustration()).unwrap();
        assert_eq!(bilan.flags, 0, "rien à flaguer sans colonne");
        assert_eq!(bilan.ajoutes, 1, "mais la ligne naît quand même");
        assert_eq!(bilan.illustrations, 1, "et reçoit son illustration");
    }
}

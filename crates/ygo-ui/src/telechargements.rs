// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les téléchargements d'images, en arrière-plan de l'interface.
//!
//! Portage de `file_attente_classeur.FileAttenteClasseur`, moins la création
//! de classeur : ici, une tâche ne fait que compléter les images d'un classeur
//! existant.
//!
//! # Pourquoi un fil séparé, et pas `async` dans la boucle de rendu
//!
//! egui redessine soixante fois par seconde. Une attente réseau dans cette
//! boucle fige la fenêtre — c'est exactement le défaut que le Python évitait
//! avec son worker. Le travail vit donc sur un fil à lui, avec sa propre
//! exécution `tokio`, et ne communique que par messages.
//!
//! L'interface ne partage **aucun état mutable** avec ce fil : elle lit une
//! file d'événements. C'est ce qui rend le service testable sans fenêtre — la
//! machine à états [`Etat`] se nourrit d'événements, d'où qu'ils viennent.
//!
//! # Le réveil de l'interface
//!
//! egui ne redessine que quand quelque chose bouge. Un événement arrivé
//! pendant que la fenêtre dort ne serait vu qu'au prochain mouvement de
//! souris — la progression semblerait figée. Le fil de travail appelle donc
//! [`egui::Context::request_repaint`] après chaque envoi ; `Context` est
//! partageable entre fils, c'est prévu pour cet usage.
//!
//! # La reprise
//!
//! `bdd/downloads_actifs.json` retient les codes dont le téléchargement est en
//! cours. Le fil le relit à son démarrage et reprend ce qui s'y trouve : une
//! fermeture pendant un téléchargement ne perd rien. Le journal ne mémorise
//! pas ce qui a été téléchargé — la reprise rescanne le disque et ne redemande
//! que ce qui manque encore.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use eframe::egui;
use ygo_core::config::Config;
use ygo_core::paths::Paths;
use ygo_images::{Journal, JournalDisque, Telechargeur};

/// Ce que l'interface demande.
enum Commande {
    /// Compléter les images de ce classeur.
    Completer(String),
    /// Télécharger des aperçus **choisis**, hors de toute passe de classeur.
    ///
    /// L'écran des artworks montre des illustrations que le classeur n'a pas
    /// encore : elles ne sont donc dans aucun plan. Elles atterrissent dans
    /// `img/small`, au même nom de fichier que la carte qu'elles deviendront
    /// une fois posées — l'aperçu n'est pas un cache à part, c'est l'image
    /// définitive, téléchargée plus tôt.
    ///
    /// `code` ne sert qu'à nommer la progression : les événements sont ceux
    /// d'une passe ordinaire, et la bande d'avancement les affiche sans rien
    /// savoir de plus.
    Apercus {
        /// Le classeur au nom duquel l'avancement s'affiche.
        code: String,
        /// Les images à chercher.
        cibles: Vec<ygo_images::plan::Cible>,
    },
    /// Reconstruire `bdd/cardinfo.db`, puis enregistrer sa version.
    ///
    /// C'est « MAJ BDD ». Elle passe par ce fil et pas par un autre : il a
    /// déjà l'exécution `tokio` et le client HTTP, et surtout il est **seul**
    /// — deux mises à jour concurrentes se disputeraient le même fichier.
    ///
    /// Elle ne porte aucun paramètre : depuis l'interface, la source est
    /// toujours le réseau. Le mode hors ligne reste au `ygo-cli`, où il sert
    /// à rejouer une initialisation sur des fixtures figées.
    Init,
    /// Redemander au serveur où en est la version de la base.
    ///
    /// Le fil le fait **de lui-même au démarrage** ; cette commande sert aux
    /// re-contrôles — après une mise à jour, typiquement, où le verdict
    /// affiché est devenu faux.
    VerifierVersion,
    /// Créer ce classeur, puis compléter ses images.
    ///
    /// L'enchaînement est celui du Python : la file d'attente créait le
    /// classeur **puis** téléchargeait ses images, dans la même tâche. Les
    /// séparer laisserait une fenêtre où le classeur existe sans une seule
    /// image.
    Creer {
        /// Le code du set.
        code: String,
        /// Faut-il compléter les variantes d'illustration via Yugipedia ?
        avec_artworks: bool,
    },
}

/// Le nom sous lequel la mise à jour de la base apparaît dans l'avancement.
///
/// Les tâches sont indexées par code de classeur ; la base n'en est pas un.
/// Un nom qu'aucun set ne peut porter — il contient des espaces — évite toute
/// collision avec un classeur réel, et se lit tel quel dans la bande de
/// progression.
pub const CODE_BASE: &str = "Base de données";

/// Où en est la mise à jour de la base.
///
/// L'initialisation émet une dizaine d'étapes typées ([`ygo_app::init::Etape`])
/// dont l'utilisateur n'a que faire : « 12 458 tirages parsés » n'aide pas à
/// patienter. On les replie sur cinq phases, qui sont les cinq attentes
/// réellement distinctes — la seule à durer plusieurs minutes est la
/// première, et c'est la seule à connaître son total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseBase {
    /// L'archive YGOJSON descend du réseau.
    Telechargement,
    /// Le catalogue YGOPRODeck descend à son tour.
    Catalogue,
    /// Cartes et sets se parsent.
    Analyse,
    /// Les variantes d'illustration se résolvent, les cartes s'enrichissent.
    Artworks,
    /// Les tables s'écrivent, en une transaction.
    Ecriture,
}

impl PhaseBase {
    /// Les cinq phases, dans l'ordre où elles se franchissent.
    ///
    /// C'est ce qui permet à un écran de les montrer **toutes** et de dire où
    /// l'on en est — plutôt qu'une seule à la fois, qui n'apprend pas combien
    /// il en reste.
    #[must_use]
    pub fn toutes() -> [Self; 5] {
        [
            Self::Telechargement,
            Self::Catalogue,
            Self::Analyse,
            Self::Artworks,
            Self::Ecriture,
        ]
    }

    /// Le rang de la phase, de 0 à 4.
    #[must_use]
    pub fn rang(self) -> usize {
        match self {
            Self::Telechargement => 0,
            Self::Catalogue => 1,
            Self::Analyse => 2,
            Self::Artworks => 3,
            Self::Ecriture => 4,
        }
    }

    /// Ce que la phase fait, en une ligne.
    #[must_use]
    pub fn detail(self) -> &'static str {
        match self {
            Self::Telechargement => "l'archive YGOJSON, environ 35 Mo",
            Self::Catalogue => "le catalogue YGOPRODeck — facultatif",
            Self::Analyse => "cartes, textes, illustrations et sets",
            Self::Artworks => "variantes d'illustration et statistiques",
            Self::Ecriture => "les tables, en une transaction",
        }
    }

    /// Le libellé montré à l'utilisateur.
    #[must_use]
    pub fn libelle(self) -> &'static str {
        match self {
            Self::Telechargement => "Téléchargement",
            Self::Catalogue => "Catalogue",
            Self::Analyse => "Analyse",
            Self::Artworks => "Artworks",
            Self::Ecriture => "Écriture",
        }
    }
}

/// Ce que le fil de travail rapporte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evenement {
    /// Une passe commence — `total` images à chercher.
    Debut {
        /// Le classeur.
        code: String,
        /// Nombre d'images à télécharger.
        total: usize,
    },
    /// Une image de plus est traitée.
    Progression {
        /// Le classeur.
        code: String,
        /// Images traitées.
        faites: usize,
        /// Total de la passe.
        total: usize,
    },
    /// La passe est finie.
    Fin {
        /// Le classeur.
        code: String,
        /// Images obtenues.
        reussies: usize,
        /// Images qu'aucune source n'a rendues.
        echecs: usize,
    },
    /// Il n'y avait rien à faire — aucun événement de début n'a été émis.
    Complet {
        /// Le classeur.
        code: String,
    },
    /// Une création commence.
    CreationEnCours {
        /// Le classeur.
        code: String,
    },
    /// La complétion des variantes d'illustration commence.
    ArtworksEnCours {
        /// Le classeur.
        code: String,
    },
    /// La couverture du classeur vient d'arriver.
    Couverture {
        /// Le classeur.
        code: String,
    },
    /// Le classeur a été créé.
    Cree {
        /// Le classeur.
        code: String,
        /// Lignes écrites.
        lignes: usize,
    },
    /// La passe n'a pas pu se faire.
    Echec {
        /// Le classeur.
        code: String,
        /// Ce qui s'est passé.
        raison: String,
    },
    /// Le serveur a dit où en est la version de la base.
    Version {
        /// Ce que le contrôle conclut.
        verdict: ygo_app::maj::Verdict,
    },
    /// La mise à jour de la base entre dans une nouvelle phase.
    BaseEnCours {
        /// Où elle en est.
        phase: PhaseBase,
    },
    /// La base est reconstruite.
    BaseTerminee {
        /// Cartes écrites.
        cartes: usize,
        /// Tirages écrits.
        tirages: usize,
        /// La version enregistrée, si le serveur l'a donnée.
        version: Option<String>,
    },
}

impl Evenement {
    /// Le classeur concerné.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::Debut { code, .. }
            | Self::Progression { code, .. }
            | Self::Fin { code, .. }
            | Self::Complet { code }
            | Self::CreationEnCours { code }
            | Self::ArtworksEnCours { code }
            | Self::Couverture { code }
            | Self::Cree { code, .. }
            | Self::Echec { code, .. } => code,
            Self::BaseEnCours { .. } | Self::BaseTerminee { .. } | Self::Version { .. } => {
                CODE_BASE
            }
        }
    }

    /// Cet événement termine-t-il une passe ?
    #[must_use]
    pub fn termine(&self) -> bool {
        matches!(
            self,
            Self::Fin { .. }
                | Self::Complet { .. }
                | Self::Echec { .. }
                | Self::BaseTerminee { .. }
        )
    }
}

/// L'avancement d'une passe en cours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Avancement {
    /// Images traitées.
    pub faites: usize,
    /// Total de la passe.
    pub total: usize,
}

/// Ce que l'interface sait des téléchargements, à un instant donné.
///
/// Machine à états **pure** : elle ne connaît ni fil, ni réseau, ni disque.
/// C'est elle qu'on éprouve.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Etat {
    en_cours: BTreeMap<String, Avancement>,
    /// Dernier message d'issue, à montrer brièvement.
    dernier: Option<String>,
    /// Classeurs dont les images viennent d'arriver, à relire par le cache.
    a_rafraichir: Vec<String>,
    /// Classeurs créés depuis la dernière lecture — l'accueil doit les voir.
    creations: Vec<String>,
    /// À quelle étape en est chaque classeur, pour l'annoncer sans mentir.
    etape: BTreeMap<String, Etape>,
    /// Ce que le serveur a répondu sur la version de la base.
    version: Option<ygo_app::maj::Verdict>,
    /// Codes demandés dont le travail n'est pas terminé — la file d'attente.
    ///
    /// `en_cours` ne porte que ce qui a **commencé** : le fil traite une
    /// commande à la fois, et trois classeurs demandés d'un coup n'y
    /// paraissent qu'un par un. Sans cette file, la bande de progression
    /// annonçait « RA05 » pendant que deux autres attendaient en silence.
    attente: std::collections::BTreeSet<String>,
    /// Tâches achevées du lot courant.
    faits: usize,
    /// L'utilisateur a écarté le bandeau — pour cette session seulement.
    ///
    /// Écarter n'est pas refuser : le contrôle reste vrai, c'est son
    /// affichage qui se tait. Au prochain démarrage il reparaît, parce qu'une
    /// base périmée le reste.
    maj_ecartee: bool,
}

/// L'étape en cours pour un classeur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Etape {
    /// Les lignes du classeur s'écrivent.
    Creation,
    /// Les variantes d'illustration se cherchent sur Yugipedia.
    Artworks,
    /// Les images se téléchargent.
    Images,
    /// La base de référence se reconstruit.
    ///
    /// Elle porte sa phase : c'est la seule tâche assez longue pour que
    /// « Travail » pendant huit minutes ne dise plus rien.
    Base(PhaseBase),
}

impl Etape {
    /// Le libellé montré à l'utilisateur.
    #[must_use]
    pub fn libelle(self) -> &'static str {
        match self {
            Self::Creation => "Création",
            Self::Artworks => "Artworks",
            Self::Images => "Images",
            Self::Base(phase) => phase.libelle(),
        }
    }
}

impl Etat {
    /// Applique un événement.
    pub fn appliquer(&mut self, evenement: &Evenement) {
        match evenement {
            Evenement::Debut { code, total } => {
                self.en_cours.insert(
                    code.clone(),
                    Avancement {
                        faites: 0,
                        total: *total,
                    },
                );
                self.etape.insert(code.clone(), Etape::Images);
            }
            Evenement::Progression {
                code,
                faites,
                total,
            } => {
                self.en_cours.insert(
                    code.clone(),
                    Avancement {
                        faites: *faites,
                        total: *total,
                    },
                );
                // Une image reçue est une carte de plus à afficher : on le
                // signale sans attendre la fin de la passe, sinon un classeur
                // de cinq cents images resterait troué jusqu'au bout.
                self.a_rafraichir.push(code.clone());
            }
            Evenement::Fin {
                code,
                reussies,
                echecs,
            } => {
                self.terminer(code);
                self.en_cours.remove(code);
                self.etape.remove(code);
                self.a_rafraichir.push(code.clone());
                self.dernier = Some(if *echecs == 0 {
                    format!("{code} — {reussies} image(s) récupérée(s)")
                } else {
                    format!("{code} — {reussies} récupérée(s), {echecs} indisponible(s)")
                });
            }
            Evenement::Complet { code } => {
                self.terminer(code);
                self.en_cours.remove(code);
                self.etape.remove(code);
            }
            Evenement::Couverture { code } => {
                // L'accueil doit relire : c'est lui qui montre les
                // couvertures, et il les charge une fois pour toutes.
                self.creations.push(code.clone());
            }
            Evenement::ArtworksEnCours { code } => {
                self.en_cours.insert(
                    code.clone(),
                    Avancement {
                        faites: 0,
                        total: 0,
                    },
                );
                self.etape.insert(code.clone(), Etape::Artworks);
            }
            Evenement::CreationEnCours { code } => {
                // Total inconnu tant que les lignes ne sont pas écrites : la
                // barre montre une attente, pas une progression mensongère.
                self.en_cours.insert(
                    code.clone(),
                    Avancement {
                        faites: 0,
                        total: 0,
                    },
                );
                self.etape.insert(code.clone(), Etape::Creation);
            }
            Evenement::Cree { code, lignes } => {
                self.creations.push(code.clone());
                self.dernier = Some(format!("{code} — classeur créé, {lignes} ligne(s)"));
            }
            Evenement::Echec { code, raison } => {
                self.terminer(code);
                self.en_cours.remove(code);
                self.etape.remove(code);
                self.dernier = Some(format!("{code} — {raison}"));
            }
            Evenement::Version { verdict } => {
                self.version = Some(verdict.clone());
            }
            Evenement::BaseEnCours { phase } => {
                // Le total repart à zéro à chaque phase : seule la première
                // en connaît un. Les suivantes montrent une barre animée
                // plutôt qu'une progression figée à 100 %.
                self.en_cours.insert(
                    CODE_BASE.to_owned(),
                    Avancement {
                        faites: 0,
                        total: 0,
                    },
                );
                self.etape.insert(CODE_BASE.to_owned(), Etape::Base(*phase));
            }
            Evenement::BaseTerminee {
                cartes,
                tirages,
                version,
            } => {
                self.en_cours.remove(CODE_BASE);
                self.etape.remove(CODE_BASE);
                // Le verdict d'avant la mise à jour ne veut plus rien dire.
                // Il est effacé ici plutôt que corrigé : le fil en redemande
                // un au serveur, et un verdict deviné n'aurait aucune valeur.
                self.version = None;
                self.maj_ecartee = false;
                // Les classeurs lisent `cardinfo.db` : ils sont tous à relire.
                // L'accueil aussi — les couvertures en dépendent.
                self.creations.push(CODE_BASE.to_owned());
                self.dernier = Some(match version {
                    Some(v) => format!(
                        "Base mise à jour — version {v}, {cartes} cartes, {tirages} tirages"
                    ),
                    None => format!(
                        "Base mise à jour — {cartes} cartes, {tirages} tirages \
                         (version du serveur indisponible)"
                    ),
                });
            }
        }
    }

    /// Y a-t-il quelque chose en cours ?
    #[must_use]
    pub fn actif(&self) -> bool {
        !self.en_cours.is_empty()
    }

    /// Les passes en cours, par code.
    #[must_use]
    pub fn en_cours(&self) -> &BTreeMap<String, Avancement> {
        &self.en_cours
    }

    /// L'avancement cumulé de toutes les passes.
    #[must_use]
    pub fn cumul(&self) -> Avancement {
        self.en_cours.values().fold(
            Avancement {
                faites: 0,
                total: 0,
            },
            |a, b| Avancement {
                faites: a.faites + b.faites,
                total: a.total + b.total,
            },
        )
    }

    /// L'étape en cours d'un classeur.
    #[must_use]
    pub fn etape(&self, code: &str) -> Option<Etape> {
        self.etape.get(code).copied()
    }

    /// Inscrit un code dans la file d'attente.
    ///
    /// Appelé à l'**envoi** de la commande, pas à son premier événement : ce
    /// qui attend son tour doit être compté, sinon la file ne dirait que ce
    /// qui a déjà commencé.
    pub fn attendre(&mut self, code: &str) {
        // Une file entièrement drainée ouvre un lot neuf : sans cela, un
        // classeur demandé une heure plus tard s'annoncerait « 7/7 ».
        if self.attente.is_empty() {
            self.faits = 0;
        }
        self.attente.insert(code.to_owned());
    }

    /// Retire un code de la file, et compte une tâche de plus.
    fn terminer(&mut self, code: &str) {
        if self.attente.remove(code) {
            self.faits += 1;
        }
    }

    /// Le lot en cours : rang de la tâche courante, et total du lot.
    ///
    /// `None` quand il n'y a rien en file, ou qu'il n'y a qu'une seule tâche —
    /// « 1/1 » n'apprend rien que la bande ne dise déjà.
    #[must_use]
    pub fn lot(&self) -> Option<(usize, usize)> {
        let total = self.faits + self.attente.len();
        (total > 1 && !self.attente.is_empty()).then_some((self.faits + 1, total))
    }

    /// Les codes qui attendent leur tour, celui en cours excepté.
    #[must_use]
    pub fn en_attente(&self) -> Vec<&str> {
        self.attente
            .iter()
            .map(String::as_str)
            .filter(|code| !self.en_cours.contains_key(*code))
            .collect()
    }

    /// La base est-elle en cours de reconstruction ?
    #[must_use]
    pub fn base_active(&self) -> bool {
        self.en_cours.contains_key(CODE_BASE)
    }

    /// Le dernier verdict du contrôle de version.
    #[must_use]
    pub fn version(&self) -> Option<&ygo_app::maj::Verdict> {
        self.version.as_ref()
    }

    /// Faut-il montrer le bandeau « mise à jour disponible » ?
    ///
    /// Trois conditions, et les trois comptent : le serveur a conclu qu'il y a
    /// quelque chose à faire, l'utilisateur n'a pas écarté le bandeau, et
    /// aucune mise à jour ne tourne — annoncer « disponible » pendant qu'elle
    /// s'installe serait absurde.
    #[must_use]
    pub fn maj_disponible(&self) -> bool {
        !self.maj_ecartee
            && !self.base_active()
            && matches!(self.version, Some(ygo_app::maj::Verdict::AFaire { .. }))
    }

    /// Écarte le bandeau pour cette session.
    pub fn ecarter_maj(&mut self) {
        self.maj_ecartee = true;
    }

    /// Oublie le verdict courant, en attendant le prochain.
    pub fn oublier_version(&mut self) {
        self.version = None;
    }

    /// Le dernier message d'issue.
    #[must_use]
    pub fn dernier(&self) -> Option<&str> {
        self.dernier.as_deref()
    }

    /// Reprend la liste des classeurs créés, et la vide.
    ///
    /// L'accueil ne se recharge qu'à ce moment-là : un classeur créé doit
    /// apparaître dans la liste sans que l'utilisateur ait à faire quoi que ce
    /// soit.
    pub fn creations(&mut self) -> Vec<String> {
        std::mem::take(&mut self.creations)
    }

    /// Reprend la liste des classeurs à rafraîchir, et la vide.
    ///
    /// L'appelant la consomme : le cache n'oublie ses fichiers absents qu'une
    /// fois par arrivée, pas à chaque image dessinée.
    pub fn a_rafraichir(&mut self) -> Vec<String> {
        std::mem::take(&mut self.a_rafraichir)
    }
}

/// Le service : un fil de travail, et la file d'événements qui en revient.
pub struct Service {
    commandes: Sender<Commande>,
    evenements: Receiver<Evenement>,
    etat: Etat,
    /// Codes déjà demandés — on ne relance pas une passe à chaque
    /// aller-retour sur le même classeur.
    demandes: std::collections::HashSet<String>,
    /// Une mise à jour de la base est partie et n'est pas revenue.
    ///
    /// Le drapeau est levé à l'**envoi**, pas au premier événement : entre
    /// les deux il y a le temps d'ouvrir une connexion HTTP, et deux clics
    /// dans cet intervalle mettraient deux reconstructions à la file.
    base_demandee: bool,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service").field("etat", &self.etat).finish()
    }
}

impl Service {
    /// Démarre le fil de travail.
    ///
    /// `ctx` sert à réveiller l'interface quand un événement arrive.
    #[must_use]
    pub fn demarrer(racine: PathBuf, ctx: egui::Context) -> Self {
        let (envoi_commandes, reception_commandes) = std::sync::mpsc::channel();
        let (envoi_evenements, reception_evenements) = std::sync::mpsc::channel();

        // Le contrôle de version part sur SON fil, pas sur celui des
        // téléchargements.
        //
        // Le fil de travail traite une commande à la fois, et commence par
        // reprendre les téléchargements interrompus. Un contrôle placé devant
        // aurait retardé cette reprise du temps de la requête — soit, hors
        // ligne, trois tentatives et deux retraits exponentiels, une bonne
        // demi-minute à chaque lancement. Placé derrière, il aurait attendu
        // que des centaines d'images finissent de descendre.
        //
        // Ce contrôle ne dépend de rien et ne bloque rien : une requête, un
        // événement, et le fil meurt. Il n'a aucune raison de partager la
        // file.
        let racine_version = racine.clone();
        let evenements_version = envoi_evenements.clone();
        let ctx_version = ctx.clone();
        std::thread::Builder::new()
            .name("ygo-version".to_owned())
            .spawn(move || controler_version(&racine_version, &evenements_version, &ctx_version))
            .map_or_else(
                |e| tracing::error!(erreur = %e, "fil de contrôle de version non démarré"),
                |_| (),
            );

        std::thread::Builder::new()
            .name("ygo-images".to_owned())
            .spawn(move || travailler(&racine, &reception_commandes, &envoi_evenements, &ctx))
            .map_or_else(
                |e| tracing::error!(erreur = %e, "fil de téléchargement non démarré"),
                |_| (),
            );

        Self {
            commandes: envoi_commandes,
            evenements: reception_evenements,
            etat: Etat::default(),
            demandes: std::collections::HashSet::new(),
            base_demandee: false,
        }
    }

    /// Demande la reconstruction de `bdd/cardinfo.db` — « MAJ BDD ».
    ///
    /// Sans effet si une mise à jour est déjà en route.
    pub fn mettre_a_jour_base(&mut self) {
        if self.base_demandee {
            return;
        }
        if self.commandes.send(Commande::Init).is_err() {
            tracing::warn!("fil de téléchargement absent — mise à jour ignorée");
            return;
        }
        self.base_demandee = true;
    }

    /// Une mise à jour de la base est-elle en cours ?
    #[must_use]
    pub fn base_en_cours(&self) -> bool {
        self.base_demandee
    }

    /// Redemande au serveur où en est la version de la base.
    ///
    /// Le fil le fait déjà à son démarrage : ceci est pour les re-contrôles.
    pub fn verifier_version(&mut self) {
        if self.commandes.send(Commande::VerifierVersion).is_err() {
            tracing::warn!("fil de téléchargement absent — contrôle de version ignoré");
            return;
        }
        // Le verdict d'avant est oublié TOUT DE SUITE : l'écran doit dire
        // « contrôle en cours » jusqu'à la réponse, et non réafficher
        // l'ancienne conclusion comme si de rien n'était.
        self.etat.oublier_version();
    }

    /// Écarte le bandeau de mise à jour pour cette session.
    pub fn ecarter_maj(&mut self) {
        self.etat.ecarter_maj();
    }

    /// Demande de créer un classeur, puis d'en compléter les images.
    pub fn creer(&mut self, code: &str, avec_artworks: bool) {
        let code = code.trim().to_uppercase();
        // La création marque aussi le code comme demandé : la passe d'images
        // qui la suit ne doit pas être relancée à l'ouverture du classeur.
        self.demandes.insert(code.clone());
        self.etat.attendre(&code);
        if self
            .commandes
            .send(Commande::Creer {
                code,
                avec_artworks,
            })
            .is_err()
        {
            tracing::warn!("fil de téléchargement absent — création ignorée");
        }
    }

    /// Demande de compléter les images d'un classeur.
    ///
    /// Sans effet si ce classeur a déjà été demandé pendant cette session :
    /// l'ouverture d'un classeur déclenche l'appel, et l'utilisateur y revient
    /// souvent.
    /// Demande le téléchargement d'aperçus.
    ///
    /// Contrairement à [`completer`](Self::completer), la demande **n'est pas
    /// dédoublonnée par classeur** : elle porte sur une liste précise, et
    /// deux appels successifs visent des images différentes.
    pub fn apercus(&mut self, code: &str, cibles: Vec<ygo_images::plan::Cible>) {
        if cibles.is_empty() {
            return;
        }
        let code = code.trim().to_uppercase();
        if self
            .commandes
            .send(Commande::Apercus { code, cibles })
            .is_err()
        {
            tracing::warn!("fil de téléchargement absent — aperçus ignorés");
        }
    }

    pub fn completer(&mut self, code: &str) {
        let code = code.trim().to_uppercase();
        if !self.demandes.insert(code.clone()) {
            return;
        }
        self.etat.attendre(&code);
        if self.commandes.send(Commande::Completer(code)).is_err() {
            tracing::warn!("fil de téléchargement absent — demande ignorée");
        }
    }

    /// Vide la file d'événements dans l'état, et rend ce qui est arrivé.
    pub fn recevoir(&mut self) -> Vec<Evenement> {
        let mut recus = Vec::new();
        // `try_recv` et non `iter` : la boucle doit rendre la main dès que la
        // file est vide, pas attendre le prochain événement. Une image bloquée
        // sur le réseau figerait sinon tout le rendu.
        while let Ok(e) = self.evenements.try_recv() {
            if e.code() == CODE_BASE && e.termine() {
                self.base_demandee = false;
            }
            self.etat.appliquer(&e);
            recus.push(e);
        }
        recus
    }

    /// L'état courant.
    #[must_use]
    pub fn etat(&self) -> &Etat {
        &self.etat
    }

    /// L'état courant, modifiable — pour reprendre la liste à rafraîchir.
    pub fn etat_mut(&mut self) -> &mut Etat {
        &mut self.etat
    }
}

/// La phase à afficher pour une étape d'initialisation.
///
/// Repli volontaire : dix étapes typées deviennent cinq attentes. `None` pour
/// celles qui ne changent pas de phase — les rapporter ferait clignoter le
/// libellé sans rien apprendre.
fn phase_de(etape: &ygo_app::init::Etape) -> Option<PhaseBase> {
    use ygo_app::init::Etape as E;
    match etape {
        E::TelechargementArchive { .. } => Some(PhaseBase::Telechargement),
        E::ArchiveRecue { .. } => Some(PhaseBase::Catalogue),
        E::CatalogueRecu { .. } | E::CatalogueIndisponible => Some(PhaseBase::Analyse),
        E::ArtworksResolus { .. } | E::CartesEnrichies { .. } => Some(PhaseBase::Artworks),
        E::EcritureBase => Some(PhaseBase::Ecriture),
        // Déjà dans la phase « Analyse » : la traverser ne la change pas.
        E::CartesParsees { .. } | E::SetsParses { .. } | E::Terminee(_) => None,
    }
}

/// La passe artworks d'un classeur qui vient de naître.
///
/// Détachée de `creation::creer` pour que la création rende la main dès que
/// les lignes sont écrites — cf. le commentaire de `Commande::Creer`.
async fn passe_artworks(
    paths: &Paths,
    client: &ygo_sources::ClientHttp,
    code: &str,
    raretes: &ygo_core::rarity::Priorites,
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = ygo_db::connexion::ouvrir(paths.classeur_db(code))?;
    ygo_app::artworks::passe(&conn, client, "", raretes).await?;
    Ok(())
}

/// Demande au serveur où en est la version de la base, une fois, puis rend la
/// main.
///
/// Un fil à lui, une requête, un événement. Hors ligne, il échoue en quelques
/// secondes et émet un [`Verdict::Indecidable`](ygo_app::maj::Verdict) — ce
/// qui n'empêche rien : le bouton de mise à jour reste actionnable, seul le
/// bandeau se tait.
fn controler_version(
    racine: &std::path::Path,
    evenements: &Sender<Evenement>,
    ctx: &egui::Context,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(erreur = %e, "contrôle de version : exécution tokio non démarrée");
            return;
        }
    };
    let client = match ygo_sources::ClientHttp::new() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(erreur = %e, "contrôle de version : client HTTP non construit");
            return;
        }
    };
    let paths = Paths::depuis_racine(racine);
    let verdict = runtime.block_on(ygo_app::maj::verifier(&paths, &client));
    let _ = evenements.send(Evenement::Version { verdict });
    ctx.request_repaint();
}

/// Exécute « MAJ BDD », en convertissant sa progression en événements.
///
/// # Pourquoi une pompe, et pas un simple `block_on`
///
/// [`ygo_app::maj::mettre_a_jour`] rend sa progression par un canal `tokio`,
/// qu'il faut vider **pendant** qu'elle travaille — sinon la barre resterait
/// figée huit minutes puis afficherait tout d'un coup. Les deux futures sont
/// donc jointes, et l'émetteur est **déplacé dans** celle du travail : sans
/// cela il resterait vivant après la fin, la pompe attendrait un message qui
/// ne viendrait jamais, et le fil ne rendrait plus la main.
fn executer_maj(
    runtime: &tokio::runtime::Runtime,
    paths: &Paths,
    client: &ygo_sources::ClientHttp,
    signaler: &impl Fn(Evenement),
) {
    signaler(Evenement::BaseEnCours {
        phase: PhaseBase::Telechargement,
    });

    let resultat = runtime.block_on(async {
        let (envoi, mut reception) = tokio::sync::mpsc::unbounded_channel();
        let options = ygo_app::init::Options::default();

        let travail = async move {
            let r = ygo_app::maj::mettre_a_jour(paths, client, &options, Some(&envoi)).await;
            drop(envoi); // ferme la pompe : sans ce `drop`, elle attendrait toujours
            r
        };

        let pompe = async {
            let mut phase = PhaseBase::Telechargement;
            while let Some(etape) = reception.recv().await {
                if let Some(nouvelle) = phase_de(&etape) {
                    if nouvelle != phase {
                        phase = nouvelle;
                        signaler(Evenement::BaseEnCours { phase });
                    }
                }
                // Seul le téléchargement connaît son total : c'est la seule
                // étape qui puisse remplir une barre sans l'inventer.
                if let ygo_app::init::Etape::TelechargementArchive { recus, total } = etape {
                    if let Some(total) = total.filter(|t| *t > 0) {
                        signaler(Evenement::Progression {
                            code: CODE_BASE.to_owned(),
                            faites: usize::try_from(recus).unwrap_or(usize::MAX),
                            total: usize::try_from(total).unwrap_or(usize::MAX),
                        });
                    }
                }
            }
        };

        let (resultat, ()) = tokio::join!(travail, pompe);
        resultat
    });

    match resultat {
        Ok(bilan) => {
            let version = bilan.version.clone();
            signaler(Evenement::BaseTerminee {
                cartes: bilan.comptages.cartes,
                tirages: bilan.comptages.tirages,
                version: bilan.version,
            });
            // Le nouveau verdict, SANS redemander au serveur.
            //
            // `maj` vient d'interroger `checkDBVer.php` pour écrire
            // `last_update.txt` : sa réponse est dans le bilan. Un second
            // appel donnait deux requêtes à la même seconde — visible dans le
            // journal du 2026-09-05 — pour reconstruire une conclusion que
            // l'on tenait déjà.
            //
            // Ce n'est pas déduire « à jour » d'un succès : c'est la version
            // que le serveur a donnée et que l'on a écrite. Quand elle
            // manque, le serveur n'a pas répondu — et là il faut bien
            // redemander.
            let verdict = match version {
                Some(version) => ygo_app::maj::Verdict::AJour { version },
                None => runtime.block_on(ygo_app::maj::verifier(paths, client)),
            };
            signaler(Evenement::Version { verdict });
        }
        Err(e) => signaler(Evenement::Echec {
            code: CODE_BASE.to_owned(),
            raison: e.to_string(),
        }),
    }
}

/// La boucle du fil de travail.
fn travailler(
    racine: &std::path::Path,
    commandes: &Receiver<Commande>,
    evenements: &Sender<Evenement>,
    ctx: &egui::Context,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(erreur = %e, "exécution tokio non démarrée");
            return;
        }
    };
    let paths = Paths::depuis_racine(racine);
    let config = Config::charger(paths.app_config());
    let source = config.source_image();
    let journal = JournalDisque::nouveau(paths.downloads_actifs());
    let raretes = ygo_core::rarity::Priorites::charger(paths.rarity_config());

    let client = match ygo_sources::ClientHttp::new() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(erreur = %e, "client HTTP non construit");
            return;
        }
    };

    let signaler = |e: Evenement| {
        let _ = evenements.send(e);
        ctx.request_repaint();
    };

    // La reprise, avant toute commande : ce qui restait en cours à la
    // fermeture précédente repart en premier.
    let mut a_faire: Vec<String> = journal.charger();
    if !a_faire.is_empty() {
        tracing::info!(classeurs = ?a_faire, "reprise des téléchargements interrompus");
    }

    loop {
        let code = if let Some(code) = a_faire.pop() {
            code
        } else {
            match commandes.recv() {
                Ok(Commande::Completer(code)) => code,
                Ok(Commande::Apercus { code, cibles }) => {
                    // Les aperçus ne passent pas par le journal de reprise :
                    // ils ne sont pas un état à rattraper, seulement un
                    // confort d'affichage. Interrompus, ils se redemandent à
                    // la réouverture de l'écran.
                    let telechargeur = Telechargeur::nouveau(&client, &paths.image_par_defaut());
                    let total = cibles.len();
                    signaler(Evenement::Debut {
                        code: code.clone(),
                        total,
                    });
                    let bilan = runtime.block_on(telechargeur.toutes(&cibles, |faites, total| {
                        signaler(Evenement::Progression {
                            code: code.clone(),
                            faites,
                            total,
                        });
                    }));
                    signaler(Evenement::Fin {
                        code,
                        reussies: bilan.reussies,
                        echecs: bilan.echecs,
                    });
                    continue;
                }
                Ok(Commande::Init) => {
                    executer_maj(&runtime, &paths, &client, &signaler);
                    continue;
                }
                Ok(Commande::VerifierVersion) => {
                    signaler(Evenement::Version {
                        verdict: runtime.block_on(ygo_app::maj::verifier(&paths, &client)),
                    });
                    continue;
                }
                Ok(Commande::Creer {
                    code,
                    avec_artworks,
                }) => {
                    signaler(Evenement::CreationEnCours { code: code.clone() });
                    // `avec_artworks: false` À DESSEIN, même quand
                    // l'utilisateur les demande.
                    //
                    // `creation::creer` enchaîne la passe artworks **avant**
                    // de rendre la main : l'événement `Cree` n'arrivait donc
                    // qu'après plusieurs minutes de réseau, et le classeur
                    // n'apparaissait à l'accueil qu'à ce moment-là. En
                    // coupant après l'écriture des lignes, la tuile apparaît
                    // en quelques secondes, et les artworks sont une étape
                    // visible de plus.
                    match runtime.block_on(ygo_app::creation::creer(
                        &paths,
                        &client,
                        &code,
                        false,
                        &ygo_app::creation::Greffons::default(),
                    )) {
                        Ok(ygo_app::creation::Issue::Cree { lignes, .. }) => {
                            // Annoncé DÈS ICI : les lignes sont écrites, le
                            // classeur existe, l'accueil peut le montrer. Ce
                            // qui suit — couverture, artworks, images — le
                            // complète mais ne conditionne plus son
                            // apparition.
                            signaler(Evenement::Cree {
                                code: code.clone(),
                                lignes,
                            });
                            // La couverture, avant les artworks : c'est ce
                            // que l'accueil montre, et elle coûte une seule
                            // requête. La voir arriver pendant que le reste
                            // travaille vaut mieux que l'attendre à la fin.
                            match runtime
                                .block_on(ygo_app::couverture::telecharger(&paths, &client, &code))
                            {
                                Ok(Some(_)) => {
                                    signaler(Evenement::Couverture { code: code.clone() })
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    tracing::warn!(classeur = %code, erreur = %e, "couverture");
                                }
                            }
                            if avec_artworks {
                                signaler(Evenement::ArtworksEnCours { code: code.clone() });
                                let bilan = runtime
                                    .block_on(passe_artworks(&paths, &client, &code, &raretes));
                                if let Err(e) = bilan {
                                    tracing::warn!(classeur = %code, erreur = %e, "passe artworks");
                                }
                            }
                        }
                        Ok(ygo_app::creation::Issue::DejaExistant) => {
                            signaler(Evenement::Echec {
                                code: code.clone(),
                                raison: "le classeur existe déjà".to_owned(),
                            });
                            continue;
                        }
                        Err(e) => {
                            signaler(Evenement::Echec {
                                code,
                                raison: e.to_string(),
                            });
                            continue;
                        }
                    }
                    // On enchaîne sur les images du classeur qui vient de
                    // naître, sans repasser par l'interface.
                    code
                }
                Err(_) => break, // l'interface est partie
            }
        };

        // La couverture avant les images : elle manque aussi aux classeurs
        // créés AVANT que la correction du code de langue existe — `LOCH-JP`
        // en est un. La chercher à chaque ouverture les rattrape, et ne coûte
        // rien quand elle est déjà là (`telecharger` sort tout de suite).
        match runtime.block_on(ygo_app::couverture::telecharger(&paths, &client, &code)) {
            Ok(Some(_)) => signaler(Evenement::Couverture { code: code.clone() }),
            Ok(None) => {}
            Err(e) => tracing::warn!(classeur = %code, erreur = %e, "couverture"),
        }

        let telechargeur = Telechargeur::nouveau(&client, &paths.image_par_defaut());
        let (cibles, _deja, _lignes) =
            match ygo_app::images::a_faire(&paths, &code, source, &telechargeur) {
                Ok(t) => t,
                Err(e) => {
                    signaler(Evenement::Echec {
                        code: code.clone(),
                        raison: e.to_string(),
                    });
                    continue;
                }
            };

        if cibles.is_empty() {
            signaler(Evenement::Complet { code });
            continue;
        }

        // Le journal n'est tenu qu'à partir d'ici : un classeur complet n'a
        // rien à reprendre, et n'a donc rien à y faire.
        journal.ajouter(&code);
        signaler(Evenement::Debut {
            code: code.clone(),
            total: cibles.len(),
        });

        let bilan = runtime.block_on(telechargeur.toutes(&cibles, |faites, total| {
            let _ = evenements.send(Evenement::Progression {
                code: code.clone(),
                faites,
                total,
            });
            ctx.request_repaint();
        }));

        journal.retirer(&code);
        signaler(Evenement::Fin {
            code,
            reussies: bilan.reussies,
            echecs: bilan.echecs,
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn debut(code: &str, total: usize) -> Evenement {
        Evenement::Debut {
            code: code.to_owned(),
            total,
        }
    }
    fn progression(code: &str, faites: usize, total: usize) -> Evenement {
        Evenement::Progression {
            code: code.to_owned(),
            faites,
            total,
        }
    }
    fn fin(code: &str, reussies: usize, echecs: usize) -> Evenement {
        Evenement::Fin {
            code: code.to_owned(),
            reussies,
            echecs,
        }
    }

    #[test]
    fn une_passe_complete_ouvre_puis_referme_l_avancement() {
        let mut etat = Etat::default();
        assert!(!etat.actif());

        etat.appliquer(&debut("RA02", 42));
        assert!(etat.actif());
        assert_eq!(
            etat.cumul(),
            Avancement {
                faites: 0,
                total: 42
            }
        );

        etat.appliquer(&progression("RA02", 10, 42));
        assert_eq!(
            etat.cumul(),
            Avancement {
                faites: 10,
                total: 42
            }
        );

        etat.appliquer(&fin("RA02", 42, 0));
        assert!(!etat.actif(), "l'avancement se referme");
        assert_eq!(etat.dernier(), Some("RA02 — 42 image(s) récupérée(s)"));
    }

    /// Deux classeurs en même temps s'additionnent : la barre montre le
    /// travail total, pas celui du dernier arrivé.
    #[test]
    fn deux_passes_simultanees_s_additionnent() {
        let mut etat = Etat::default();
        etat.appliquer(&debut("RA02", 42));
        etat.appliquer(&debut("RA05", 8));
        etat.appliquer(&progression("RA02", 10, 42));
        etat.appliquer(&progression("RA05", 3, 8));
        assert_eq!(
            etat.cumul(),
            Avancement {
                faites: 13,
                total: 50
            }
        );
        assert_eq!(etat.en_cours().len(), 2);

        etat.appliquer(&fin("RA02", 42, 0));
        assert_eq!(
            etat.cumul(),
            Avancement {
                faites: 3,
                total: 8
            }
        );
        assert!(etat.actif(), "RA05 continue");
    }

    /// Un classeur déjà complet ne doit rien afficher du tout — ni barre, ni
    /// message. C'est le cas de huit classeurs sur neuf à chaque ouverture.
    #[test]
    fn un_classeur_complet_ne_montre_rien() {
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::Complet {
            code: "RA05".to_owned(),
        });
        assert!(!etat.actif());
        assert_eq!(etat.dernier(), None, "aucun message pour un non-événement");
        assert!(etat.a_rafraichir().is_empty(), "rien de neuf à relire");
    }

    #[test]
    fn un_echec_referme_l_avancement_et_le_dit() {
        let mut etat = Etat::default();
        etat.appliquer(&debut("RA02", 42));
        etat.appliquer(&Evenement::Echec {
            code: "RA02".to_owned(),
            raison: "base illisible".to_owned(),
        });
        assert!(!etat.actif());
        assert_eq!(etat.dernier(), Some("RA02 — base illisible"));
    }

    #[test]
    fn le_message_distingue_les_indisponibles() {
        let mut etat = Etat::default();
        etat.appliquer(&fin("RA02", 40, 2));
        assert_eq!(
            etat.dernier(),
            Some("RA02 — 40 récupérée(s), 2 indisponible(s)")
        );
    }

    /// Le rafraîchissement se demande **à chaque image**, pas seulement à la
    /// fin : un classeur de cinq cents images resterait troué jusqu'au bout.
    #[test]
    fn chaque_image_recue_demande_un_rafraichissement() {
        let mut etat = Etat::default();
        etat.appliquer(&debut("RA02", 3));
        etat.appliquer(&progression("RA02", 1, 3));
        etat.appliquer(&progression("RA02", 2, 3));
        etat.appliquer(&fin("RA02", 3, 0));
        assert_eq!(etat.a_rafraichir().len(), 3, "deux images plus la fin");
    }

    /// La liste se consomme : le cache n'oublie ses absents qu'une fois par
    /// arrivée, pas à chaque image dessinée.
    #[test]
    fn la_liste_a_rafraichir_se_consomme() {
        let mut etat = Etat::default();
        etat.appliquer(&progression("RA02", 1, 3));
        assert_eq!(etat.a_rafraichir(), ["RA02"]);
        assert!(etat.a_rafraichir().is_empty(), "une seule fois");
    }

    /// Une création annonce son début sans total — on ne sait pas encore
    /// combien de lignes le set fera — puis se conclut par `Cree`.
    #[test]
    fn une_creation_ouvre_un_avancement_sans_total_puis_se_conclut() {
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::CreationEnCours {
            code: "RA02".to_owned(),
        });
        assert!(etat.actif());
        assert_eq!(
            etat.cumul(),
            Avancement {
                faites: 0,
                total: 0
            }
        );

        etat.appliquer(&Evenement::Cree {
            code: "RA02".to_owned(),
            lignes: 553,
        });
        assert_eq!(etat.dernier(), Some("RA02 — classeur créé, 553 ligne(s)"));
    }

    /// L'accueil ne se recharge qu'une fois par création : la liste se
    /// **consomme**, sinon la lecture des neuf classeurs repartirait à chaque
    /// image dessinée.
    #[test]
    fn les_creations_se_consomment() {
        let mut etat = Etat::default();
        assert!(etat.creations().is_empty());

        etat.appliquer(&Evenement::Cree {
            code: "RA02".to_owned(),
            lignes: 553,
        });
        assert_eq!(etat.creations(), ["RA02"]);
        assert!(etat.creations().is_empty(), "une seule fois");
    }

    /// L'étape est nommée à chaque instant : sans elle, la bande annonçait
    /// « Images » pendant que le classeur s'écrivait encore.
    #[test]
    fn l_etape_suit_le_travail_reel() {
        let mut etat = Etat::default();
        assert_eq!(etat.etape("RA02"), None);

        etat.appliquer(&Evenement::CreationEnCours {
            code: "RA02".to_owned(),
        });
        assert_eq!(etat.etape("RA02"), Some(Etape::Creation));

        etat.appliquer(&Evenement::ArtworksEnCours {
            code: "RA02".to_owned(),
        });
        assert_eq!(etat.etape("RA02"), Some(Etape::Artworks));

        etat.appliquer(&debut("RA02", 42));
        assert_eq!(etat.etape("RA02"), Some(Etape::Images));

        etat.appliquer(&fin("RA02", 42, 0));
        assert_eq!(etat.etape("RA02"), None, "l'étape s'efface avec la tâche");
    }

    /// Les trois étapes ont des libellés distincts — sinon la bande dirait la
    /// même chose du début à la fin.
    #[test]
    fn les_trois_etapes_ont_des_libelles_distincts() {
        let mut libelles = [
            Etape::Creation.libelle(),
            Etape::Artworks.libelle(),
            Etape::Images.libelle(),
        ];
        libelles.sort_unstable();
        let avant = libelles.len();
        let mut sans_doublon = libelles.to_vec();
        sans_doublon.dedup();
        assert_eq!(sans_doublon.len(), avant);
    }

    /// Un échec ou un classeur complet efface aussi l'étape : la bande ne doit
    /// pas garder un libellé orphelin.
    #[test]
    fn l_etape_s_efface_aussi_sur_echec_et_sur_complet() {
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::CreationEnCours {
            code: "RA02".to_owned(),
        });
        etat.appliquer(&Evenement::Echec {
            code: "RA02".to_owned(),
            raison: "x".to_owned(),
        });
        assert_eq!(etat.etape("RA02"), None);

        etat.appliquer(&debut("RA05", 3));
        etat.appliquer(&Evenement::Complet {
            code: "RA05".to_owned(),
        });
        assert_eq!(etat.etape("RA05"), None);
    }

    /// La création enchaîne sur les images du même classeur : l'avancement
    /// passe du total inconnu au total réel sans se refermer entre-temps.
    #[test]
    fn la_creation_enchaine_sur_les_images_sans_se_refermer() {
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::CreationEnCours {
            code: "RA02".to_owned(),
        });
        etat.appliquer(&Evenement::Cree {
            code: "RA02".to_owned(),
            lignes: 553,
        });
        assert!(etat.actif(), "la tâche continue vers les images");

        etat.appliquer(&debut("RA02", 123));
        assert_eq!(
            etat.cumul(),
            Avancement {
                faites: 0,
                total: 123
            }
        );
        etat.appliquer(&fin("RA02", 123, 0));
        assert!(!etat.actif());
    }

    #[test]
    fn chaque_evenement_nomme_son_classeur_et_dit_s_il_termine() {
        assert_eq!(debut("RA02", 1).code(), "RA02");
        assert!(!debut("RA02", 1).termine());
        assert!(!progression("RA02", 1, 2).termine());
        assert!(fin("RA02", 1, 0).termine());
        assert!(Evenement::Complet {
            code: "RA02".to_owned()
        }
        .termine());
        assert!(Evenement::Echec {
            code: "RA02".to_owned(),
            raison: String::new()
        }
        .termine());
        // Une création n'est pas une fin : les images suivent.
        assert!(!Evenement::CreationEnCours {
            code: "RA02".to_owned()
        }
        .termine());
        assert!(!Evenement::Cree {
            code: "RA02".to_owned(),
            lignes: 1
        }
        .termine());
    }

    /// La mise à jour de la base se comporte comme une tâche parmi les
    /// autres : elle ouvre l'avancement, le tient, et le referme.
    #[test]
    fn la_mise_a_jour_de_la_base_ouvre_puis_referme_l_avancement() {
        let mut etat = Etat::default();
        assert!(!etat.base_active());

        etat.appliquer(&Evenement::BaseEnCours {
            phase: PhaseBase::Telechargement,
        });
        assert!(etat.actif());
        assert!(etat.base_active());
        assert_eq!(
            etat.etape(CODE_BASE),
            Some(Etape::Base(PhaseBase::Telechargement))
        );

        etat.appliquer(&progression(CODE_BASE, 1_048_576, 170_000_000));
        assert_eq!(
            etat.cumul(),
            Avancement {
                faites: 1_048_576,
                total: 170_000_000
            }
        );

        etat.appliquer(&Evenement::BaseEnCours {
            phase: PhaseBase::Ecriture,
        });
        assert_eq!(
            etat.etape(CODE_BASE),
            Some(Etape::Base(PhaseBase::Ecriture)),
            "le libellé suit la phase"
        );
        assert_eq!(
            etat.cumul(),
            Avancement {
                faites: 0,
                total: 0
            },
            "une phase sans total connu remet la barre à l'animation"
        );

        etat.appliquer(&Evenement::BaseTerminee {
            cartes: 13_500,
            tirages: 320_000,
            version: Some("146.68".to_owned()),
        });
        assert!(!etat.actif());
        assert!(!etat.base_active());
        let dernier = etat.dernier().unwrap_or_default();
        assert!(dernier.contains("146.68"), "{dernier}");
    }

    /// La base reconstruite est une nouveauté pour l'accueil : tous les
    /// classeurs la relisent.
    #[test]
    fn une_base_reconstruite_fait_recharger_l_accueil() {
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::BaseTerminee {
            cartes: 1,
            tirages: 1,
            version: None,
        });
        assert_eq!(etat.creations(), vec![CODE_BASE.to_owned()]);
    }

    /// Sans version, on le dit — plutôt que de laisser croire à un succès
    /// complet.
    #[test]
    fn une_version_manquante_est_annoncee() {
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::BaseTerminee {
            cartes: 1,
            tirages: 1,
            version: None,
        });
        let dernier = etat.dernier().unwrap_or_default();
        assert!(dernier.contains("version"), "{dernier}");
    }

    /// Un échec de mise à jour se referme comme les autres, sous le même nom.
    #[test]
    fn un_echec_de_mise_a_jour_referme_l_avancement() {
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::BaseEnCours {
            phase: PhaseBase::Telechargement,
        });
        etat.appliquer(&Evenement::Echec {
            code: CODE_BASE.to_owned(),
            raison: "GitHub injoignable".to_owned(),
        });
        assert!(!etat.base_active());
        assert!(!etat.actif());
    }

    /// Les deux événements de base répondent `code()` sans qu'on ait à le
    /// leur passer — c'est ce qui permet au service de reconnaître la fin.
    #[test]
    fn les_evenements_de_base_se_nomment_seuls() {
        assert_eq!(
            Evenement::BaseEnCours {
                phase: PhaseBase::Analyse
            }
            .code(),
            CODE_BASE
        );
        assert!(!Evenement::BaseEnCours {
            phase: PhaseBase::Analyse
        }
        .termine());
        assert!(Evenement::BaseTerminee {
            cartes: 0,
            tirages: 0,
            version: None
        }
        .termine());
    }

    /// Chaque phase a son libellé, et aucun n'est vide : la bande de
    /// progression les affiche tels quels.
    #[test]
    fn les_phases_ont_des_libelles_distincts() {
        let phases = [
            PhaseBase::Telechargement,
            PhaseBase::Catalogue,
            PhaseBase::Analyse,
            PhaseBase::Artworks,
            PhaseBase::Ecriture,
        ];
        let mut vus = std::collections::HashSet::new();
        for phase in phases {
            let libelle = Etape::Base(phase).libelle();
            assert!(!libelle.is_empty());
            assert!(vus.insert(libelle), "libellé répété : {libelle}");
        }
    }

    fn a_faire(locale: Option<&str>, distante: &str) -> Evenement {
        Evenement::Version {
            verdict: ygo_app::maj::Verdict::AFaire {
                locale: locale.map(ToOwned::to_owned),
                distante: distante.to_owned(),
            },
        }
    }

    /// Le bandeau ne paraît que sur un verdict qui dit qu'il y a à faire.
    #[test]
    fn le_bandeau_suit_le_verdict() {
        let mut etat = Etat::default();
        assert!(
            !etat.maj_disponible(),
            "rien tant que le serveur n'a rien dit"
        );

        etat.appliquer(&Evenement::Version {
            verdict: ygo_app::maj::Verdict::AJour {
                version: "146.68".to_owned(),
            },
        });
        assert!(!etat.maj_disponible(), "à jour ne se signale pas");

        etat.appliquer(&Evenement::Version {
            verdict: ygo_app::maj::Verdict::Indecidable {
                raison: "serveur injoignable".to_owned(),
            },
        });
        assert!(
            !etat.maj_disponible(),
            "indécidable ne se signale pas non plus"
        );

        etat.appliquer(&a_faire(Some("146.68"), "147.01"));
        assert!(etat.maj_disponible());
    }

    /// Annoncer « disponible » pendant que la mise à jour s'installe serait
    /// absurde : le bandeau se tait dès que le travail commence.
    #[test]
    fn le_bandeau_se_tait_pendant_la_mise_a_jour() {
        let mut etat = Etat::default();
        etat.appliquer(&a_faire(Some("146.68"), "147.01"));
        assert!(etat.maj_disponible());

        etat.appliquer(&Evenement::BaseEnCours {
            phase: PhaseBase::Telechargement,
        });
        assert!(!etat.maj_disponible(), "pas pendant le travail");
    }

    /// Écarter le bandeau ne change pas le verdict — c'est son affichage qui
    /// se tait.
    #[test]
    fn ecarter_le_bandeau_garde_le_verdict() {
        let mut etat = Etat::default();
        etat.appliquer(&a_faire(None, "147.01"));
        etat.ecarter_maj();
        assert!(!etat.maj_disponible());
        assert!(
            matches!(etat.version(), Some(ygo_app::maj::Verdict::AFaire { .. })),
            "le contrôle reste vrai"
        );
    }

    /// Après une reconstruction, le verdict d'avant ne veut plus rien dire :
    /// il est effacé, pas deviné. Le fil en redemande un au serveur.
    #[test]
    fn une_mise_a_jour_efface_le_verdict_precedent() {
        let mut etat = Etat::default();
        etat.appliquer(&a_faire(Some("146.68"), "147.01"));
        etat.ecarter_maj();

        etat.appliquer(&Evenement::BaseTerminee {
            cartes: 13_500,
            tirages: 320_000,
            version: Some("147.01".to_owned()),
        });
        assert_eq!(etat.version(), None, "effacé, pas déduit");
        assert!(!etat.maj_disponible());

        // Et le nouveau verdict repart d'une ardoise propre : écarter le
        // bandeau une fois ne le condamne pas pour la session entière.
        etat.appliquer(&a_faire(Some("147.01"), "147.02"));
        assert!(etat.maj_disponible());
    }

    /// Le contrôle de version se nomme comme la base, et ne termine rien —
    /// sans quoi il libérerait la garde anti double-lancement.
    #[test]
    fn le_controle_de_version_ne_termine_rien() {
        let evenement = a_faire(None, "147.01");
        assert_eq!(evenement.code(), CODE_BASE);
        assert!(!evenement.termine());
    }

    /// Le retour d'usage du 2026-09-05 : trois classeurs demandés d'un coup,
    /// et la bande n'en annonçait qu'un.
    #[test]
    fn la_file_compte_ce_qui_attend_son_tour() {
        let mut etat = Etat::default();
        assert_eq!(etat.lot(), None, "rien demandé, rien à dire");

        for code in ["RA05", "LOCR-JP", "LOCH-JP"] {
            etat.attendre(code);
        }
        assert_eq!(etat.lot(), Some((1, 3)));

        // Le fil n'en traite qu'un : les deux autres attendent.
        etat.appliquer(&debut("RA05", 156));
        assert_eq!(etat.lot(), Some((1, 3)));
        assert_eq!(etat.en_attente(), vec!["LOCH-JP", "LOCR-JP"]);

        etat.appliquer(&fin("RA05", 156, 0));
        assert_eq!(etat.lot(), Some((2, 3)), "le rang avance");

        etat.appliquer(&debut("LOCH-JP", 300));
        etat.appliquer(&fin("LOCH-JP", 300, 0));
        assert_eq!(etat.lot(), Some((3, 3)));

        etat.appliquer(&debut("LOCR-JP", 10));
        etat.appliquer(&fin("LOCR-JP", 10, 0));
        assert_eq!(etat.lot(), None, "le lot fini ne s'affiche plus");
    }

    /// « 1/1 » n'apprend rien que la bande ne dise déjà.
    #[test]
    fn une_tache_seule_ne_montre_pas_de_rang() {
        let mut etat = Etat::default();
        etat.attendre("RA05");
        assert_eq!(etat.lot(), None);
        etat.appliquer(&debut("RA05", 42));
        assert_eq!(etat.lot(), None);
    }

    /// Un classeur demandé une heure après le lot précédent ne doit pas
    /// s'annoncer « 7/7 » : une file drainée ouvre un lot neuf.
    #[test]
    fn une_file_videe_ouvre_un_lot_neuf() {
        let mut etat = Etat::default();
        etat.attendre("A");
        etat.attendre("B");
        etat.appliquer(&fin("A", 1, 0));
        etat.appliquer(&fin("B", 1, 0));
        assert_eq!(etat.lot(), None);

        etat.attendre("C");
        etat.attendre("D");
        assert_eq!(etat.lot(), Some((1, 2)), "et non (3, 4)");
    }

    /// Un classeur complet n'a rien à télécharger : il sort de la file comme
    /// les autres, sinon elle ne se viderait jamais.
    #[test]
    fn un_classeur_complet_sort_de_la_file() {
        let mut etat = Etat::default();
        etat.attendre("RA05");
        etat.attendre("VASM");
        etat.appliquer(&Evenement::Complet {
            code: "RA05".to_owned(),
        });
        assert_eq!(etat.lot(), Some((2, 2)));
        assert_eq!(etat.en_attente(), vec!["VASM"]);
    }

    /// Un échec aussi : sans quoi une passe ratée bloquerait le compteur.
    #[test]
    fn un_echec_sort_de_la_file() {
        let mut etat = Etat::default();
        etat.attendre("RA05");
        etat.attendre("VASM");
        etat.appliquer(&Evenement::Echec {
            code: "RA05".to_owned(),
            raison: "réseau".to_owned(),
        });
        assert_eq!(etat.lot(), Some((2, 2)));
    }
}

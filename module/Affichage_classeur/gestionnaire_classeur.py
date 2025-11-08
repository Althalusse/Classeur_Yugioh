import os
import sqlite3
import tkinter as tk
from tkinter import ttk, messagebox
from module.centralisation_dossier import AFFICHER_CARTE, IMG_FOLDER
from module.img_dl import gui_telechargement
from module.img_dl.telechargement_service import TelechargementService
from module.gestion_img.gestion_image_classeur import (
    get_image_path,
    load_image,
    get_placeholder_image,
    # open_full_image  # NE PAS importer ici pour éviter l'import circulaire
)
from module.gestion_rarete.tri_carte import sort_cartes
from module.carte_posseder.gestion_carte_posseder import (update_possessed_in_db, set_all_possessed)
from module.carte_posseder.affichage_carte_classeur import get_cartes_info
from module.utilitaire.actualisation_UI import creer_callback_rafraichir
from module.recherche_carte import recherche_carte

# NE PAS importer open_full_image ni rien de visualiseur_classeur_ui ici

def verifier_et_telecharger_images_manquantes(code_set, liste_cartes):
    images_manquantes = False
    for carte in liste_cartes:
        nom_image = f"{carte['code']}.jpg"
        chemin_img = os.path.join(IMG_FOLDER, code_set, nom_image)
        if not os.path.exists(chemin_img):
            images_manquantes = True
            break
    if images_manquantes:
        # Correction : instanciation de TelechargementGUI
        service = TelechargementService()
        manager = None  # Si vous avez un manager, passez-le ici, sinon None
        tele_gui = gui_telechargement.TelechargementGUI(service, manager)
        tele_gui.telecharger_images_gui(code_set)


def obtenir_liste_classeurs():
    if not os.path.exists(AFFICHER_CARTE):
        return []
    return [
        d
        for d in os.listdir(AFFICHER_CARTE)
        if os.path.isdir(os.path.join(AFFICHER_CARTE, d))
    ]


def afficher_cartes_sur_cadre(
    parent_frame,
    cartes,
    colonnes,
    images_cache,
    selected_classeur_var,
    refresh_stats_callback=None,
):
    # Import local pour éviter l'import circulaire
    from module.Affichage_classeur.visualiseur_classeur_ui import open_full_image
    for i, carte in enumerate(cartes):
        ligne = i // colonnes
        colonne = i % colonnes
        img_path = get_image_path(selected_classeur_var.get(), carte["image_filename"])
        img_tk = load_image(img_path)
        if img_tk is None:
            img_tk = get_placeholder_image()
        images_cache.append(img_tk)
        carte_frame = ttk.Frame(
            parent_frame, relief=tk.RAISED, borderwidth=1, width=200, height=250
        )
        carte_frame.grid(row=ligne, column=colonne, padx=5, pady=5, sticky="nsew")
        carte_frame.grid_propagate(False)
        label_img = tk.Label(carte_frame, image=img_tk)
        label_img.pack()
        label_img.bind(
            "<Button-1>", lambda e, p=img_path, n=carte["name"]: open_full_image(p, n)
        )
        lbl_name = tk.Label(
            carte_frame, text=carte["name"], wraplength=190, justify="center"
        )
        lbl_name.pack()
        lbl_name.config(height=2)
        lbl_rarity = tk.Label(
            carte_frame,
            text=f"Rareté: {carte['rarity']}",
            wraplength=190,
            justify="center",
        )
        lbl_rarity.pack()
        lbl_rarity.config(height=1)
        lbl_code = tk.Label(carte_frame, text=f"Code: {carte['code']}")
        lbl_code.pack()
        lbl_code.config(height=1)
        var_possede = tk.IntVar(value=carte.get("possessed", 0))
        chk = ttk.Checkbutton(carte_frame, text="Possédée", variable=var_possede)
        chk.pack()

        def lors_changement_checkbox(
            var=var_possede, set_code=carte["set_code"], set_rarity=carte["set_rarity"]
        ):
            update_possessed_in_db(
                os.path.join(
                    AFFICHER_CARTE,
                    selected_classeur_var.get(),
                    f"{selected_classeur_var.get()}.db",
                ),
                set_code,
                set_rarity,
                var.get(),
            )
            if refresh_stats_callback:
                # Utilise le callback centralisé si besoin
                creer_callback_rafraichir(refresh_stats_callback)()

        var_possede.trace_add(
            "write",
            lambda *args, v=var_possede, s=carte["set_code"], r=carte[
                "set_rarity"
            ]: lors_changement_checkbox(v, s, r),
        )
        # Bouton Cardmarket retiré


__all__ = [
    "verifier_et_telecharger_images_manquantes",
    "obtenir_liste_classeurs",
    "afficher_cartes_sur_cadre",
]

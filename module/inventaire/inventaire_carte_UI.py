import tkinter as tk
from tkinter import ttk, messagebox
import os
import sqlite3
from . import inventaire_carte
# Corrige les imports relatifs pour update_quantite et theme
from module.update_quantite import update_quantite_in_classeur, update_qualite_in_classeur
from module.theme import YugiohTheme

def creer_interface_inventaire(parent):
    # Appliquer le thème
    YugiohTheme.setup()
    
    """
    Crée et retourne le cadre principal de l'interface d'inventaire des cartes.
    """
    # Création du layout principal avec sidebar
    main_panel = ttk.PanedWindow(parent, orient=tk.HORIZONTAL)
    main_panel.pack(fill=tk.BOTH, expand=True)

    # Sidebar pour les filtres
    sidebar = ttk.Frame(main_panel, width=220)
    main_panel.add(sidebar, weight=0)
    sidebar.pack_propagate(False)

    # Zone principale pour le tableau
    content = ttk.Frame(main_panel)
    main_panel.add(content, weight=1)

    # Récupère toutes les cartes possédées avec rareté et display
    def get_cartes():
        cartes = inventaire_carte.get_cartes_possedees()
        raretes = set()
        displays = set()
        for carte in cartes:
            raretes.add(carte.get("card_sets_set_rarity", ""))
            displays.add(carte.get("card_sets_set_name", ""))
        return cartes, sorted([r for r in raretes if r]), sorted([d for d in displays if d])

    # Filtres dynamiques
    sidebar_heading = ttk.Label(
        sidebar,
        text="Filtres d'inventaire",
        style="Title.TLabel"
    )
    sidebar_heading.pack(pady=(10, 5))
    
    # Zone des filtres améliorée
    filter_frame = ttk.LabelFrame(sidebar, text="Filtres", padding="10")
    filter_frame.pack(fill="x", padx=10, pady=5)

    # Groupe Rareté
    rarity_frame = ttk.Frame(filter_frame)
    rarity_frame.pack(fill="x", pady=5)
    ttk.Label(rarity_frame, text="🏆 Rareté:", style="Title.TLabel", anchor="w").pack(fill="x")
    combo_rarete = ttk.Combobox(rarity_frame, state="readonly")
    combo_rarete.pack(fill="x", pady=(2, 0))

    # Groupe Code Carte
    code_frame = ttk.Frame(filter_frame)
    code_frame.pack(fill="x", pady=5)
    ttk.Label(code_frame, text="🔍 Code carte:", style="Title.TLabel", anchor="w").pack(fill="x")
    entry_code = ttk.Entry(code_frame)
    entry_code.pack(fill="x", pady=(2, 0))

    # Groupe Display
    display_frame = ttk.Frame(filter_frame)
    display_frame.pack(fill="x", pady=5)
    ttk.Label(display_frame, text="📦 Display (Set):", style="Title.TLabel", anchor="w").pack(fill="x")
    combo_display = ttk.Combobox(display_frame, state="readonly")
    combo_display.pack(fill="x", pady=(2, 0))

    # Boutons avec style
    btn_frame = ttk.Frame(filter_frame)
    btn_frame.pack(fill="x", pady=10)
    
    btn_filtrer = ttk.Button(btn_frame, text="🔍 Filtrer", style="Primary.TButton")
    btn_filtrer.pack(side="left", padx=2, fill="x", expand=True)
    
    btn_reset = ttk.Button(btn_frame, text="↺ Réinitialiser", style="Primary.TButton")
    btn_reset.pack(side="right", padx=2, fill="x", expand=True)

    # Correction : définir tree_columns AVANT de l'utiliser (évite l'erreur "tree_columns is not defined")
    tree_columns = {
        "name": ("Nom de carte", 200),
        "set_name": ("Display (Set)", 150),
        "set_code": ("Code carte", 100),
        "quantite": ("Quantité", 80),
        "qualite": ("Qualité", 100),
        "classeur": ("Classeur", 100),
        "rarete": ("Rareté", 100)
    }

    # Tableau principal avec la nouvelle colonne qualité
    tree = ttk.Treeview(content, style="Yugioh.Treeview",
                       columns=tuple(tree_columns.keys()),
                       show="headings", selectmode="extended")
    # Ajout d'une barre de défilement verticale CORRECTE
    yscroll = ttk.Scrollbar(content, orient="vertical", command=tree.yview)
    xscroll = ttk.Scrollbar(content, orient="horizontal", command=tree.xview)
    tree.configure(yscrollcommand=yscroll.set, xscrollcommand=xscroll.set)
    tree.grid(row=0, column=0, sticky="nsew")
    yscroll.grid(row=0, column=1, sticky="ns")
    xscroll.grid(row=1, column=0, sticky="ew")
    content.grid_rowconfigure(0, weight=1)
    content.grid_columnconfigure(0, weight=1)

    # Tri des colonnes par clic sur l'en-tête
    def sort_treeview(col, reverse=False):
        def get_val(iid):
            val = tree.set(iid, col)
            # Correction stricte : si val n'est pas une chaîne ou un nombre, retourne une valeur neutre
            if col == "quantite":
                try:
                    # Si val est déjà un int ou str convertible, ok, sinon retourne 0
                    return int(val) if isinstance(val, (str, int, float)) and str(val).isdigit() else 0
                except Exception:
                    return 0
            return str(val).lower() if isinstance(val, str) else (str(val) if val is not None else "")
        data = [(get_val(k), k) for k in tree.get_children('')]
        data.sort(reverse=reverse)
        for index, (val, k) in enumerate(data):
            tree.move(k, '', index)
        tree.heading(col, command=lambda: sort_treeview(col, not reverse))

    for col, (text, width) in tree_columns.items():
        tree.heading(col, text=text, command=lambda c=col: sort_treeview(c, False))
        tree.column(col, width=width, minwidth=50)

    # Supprimer tous les tree.pack(...) et xscroll.pack(...) inutiles
    # (ne pas mélanger pack et grid dans le même conteneur !)

    # Style pour le Treeview
    tree_style = ttk.Style()
    tree_style.configure(
        "Custom.Treeview",
        background="white",
        foreground="black",
        fieldbackground="white",
        rowheight=30
    )
    tree.configure(style="Custom.Treeview")

    # Style pour la sélection (surlignage)
    style = ttk.Style()
    style.map("Custom.Treeview", background=[("selected", "#cce6ff")])
    tree.configure(style="Custom.Treeview")

    # Edition directe de la quantité (double-clic)
    def on_double_click(event):
        item = tree.identify_row(event.y)
        column = tree.identify_column(event.x)
        
        if not item or column not in ("#4", "#5"):
            return
        
        values = tree.item(item, 'values')
        if not values or len(values) < 7:
            return
            
        nom_carte = values[0]
        set_code = values[2]
        valeur_actuelle = values[3] if column == "#4" else values[4]
        classeur = values[5]
        rarete = values[6]
        
        x, y, width, height = tree.bbox(item, column)
        
        if column == "#5":  # Colonne qualité
            # Créer une combobox pour la qualité
            combo = ttk.Combobox(tree, values=QUALITE_VALUES, state="readonly")
            combo.place(x=x, y=y, width=width, height=height)
            if valeur_actuelle in QUALITE_VALUES:
                combo.set(valeur_actuelle)
            
            def save_quality(event=None):
                new_quality = combo.get()
                if update_qualite_in_classeur(classeur, nom_carte, set_code, new_quality, rarete):
                    combo.destroy()

            
            combo.bind('<<ComboboxSelected>>', save_quality)
            combo.bind('<Escape>', lambda e: combo.destroy())
            combo.focus_set()
            
        else:  # Colonne quantité
            # Créer l'Entry pour l'édition
            entry = ttk.Entry(tree)
            entry.place(x=x, y=y, width=width, height=height)
            entry.insert(0, valeur_actuelle)
            entry.select_range(0, tk.END)
            entry.focus_set()
            
            def save_edit():
                try:
                    nouvelle_valeur = entry.get()
                    if column == "#4":  # Quantité
                        nouvelle_valeur = int(nouvelle_valeur)
                        if nouvelle_valeur < 1:
                            messagebox.showwarning("Erreur", "La quantité doit être supérieure à 0")
                            entry.focus_set()
                            return
                        success = update_quantite_in_classeur(classeur, nom_carte, set_code, nouvelle_valeur, rarete)
                    else:
                        success = True
                    if success:
                        entry.destroy()
                except ValueError:
                    if column == "#4":
                        messagebox.showwarning("Erreur", "Veuillez entrer un nombre entier valide")
                    entry.focus_set()

            def on_focus_out(event):
                # Ne rien faire si le focus va vers une fenêtre de message
                if isinstance(event.widget, (tk.Toplevel, tk.Tk)):
                    return
                # Sauvegarder si l'utilisateur clique ailleurs
                if event.widget != entry:
                    save_edit()
            
            def on_escape(event):
                entry.destroy()
            
            # Lier les événements
            entry.bind('<Return>', lambda e: save_edit())
            entry.bind('<Escape>', lambda e: on_escape(e))
            entry.bind('<FocusOut>', on_focus_out)
    
    tree.bind("<Double-1>", on_double_click)
    

    # Menu contextuel (clic droit)
    QUALITE_VALUES = ["Mint","Near Mint", "Excellent", "Bon", "Moyen", "joué", "Abîmé"]

    menu = tk.Menu(tree, tearoff=0)
    qualite_menu = tk.Menu(menu, tearoff=0)
    for qualite in QUALITE_VALUES:
        qualite_menu.add_command(
            label=qualite,
            command=lambda q=qualite: set_quality_for_selection(q)
        )
    menu.add_cascade(label="Définir la qualité", menu=qualite_menu)
    menu.add_separator()
    menu.add_command(label="Définir la quantité...", command=lambda: set_quantity_for_selection())

    def show_context_menu(event):
        iid = tree.identify_row(event.y)
        # Correction : sélectionne la ligne sous la souris AVANT d'ouvrir le menu
        if iid:
            if iid not in tree.selection():
                tree.selection_set(iid)
        else:
            # Si clic droit hors d'une ligne, ne rien faire
            return
        menu.tk_popup(event.x_root, event.y_root)

    tree.bind("<Button-3>", show_context_menu)

    def set_quality_for_selection(qualite):
        # Correction : applique la qualité et met à jour l'affichage dans le treeview
        for iid in tree.selection():
            vals = tree.item(iid, 'values')
            if not vals or len(vals) < 7:
                continue
            nom_carte = vals[0]
            set_code = vals[2]
            classeur = vals[5]
            rarete = vals[6]
            if update_qualite_in_classeur(classeur, nom_carte, set_code, qualite, rarete):
                # Met à jour la valeur dans le treeview
                new_vals = list(vals)
                new_vals[4] = qualite
                tree.item(iid, values=new_vals)

    def set_quantity_for_selection():
        qty_win = tk.Toplevel(tree)
        qty_win.title("Définir la quantité")
        qty_win.geometry("250x100")
        root = tree.winfo_toplevel()
        qty_win.transient(root)
        qty_win.grab_set()
        tk.Label(qty_win, text="Nouvelle quantité :").pack(pady=5)
        entry = ttk.Entry(qty_win)
        entry.pack(pady=5)
        entry.focus_set()
        def apply_qty():
            try:
                val = int(entry.get())
                if val < 1:
                    messagebox.showwarning("Erreur", "La quantité doit être supérieure à 0")
                    return
                for iid in tree.selection():
                    vals = tree.item(iid, 'values')
                    if not vals or len(vals) < 7:
                        continue
                    nom_carte = vals[0]
                    set_code = vals[2]
                    classeur = vals[5]
                    rarete = vals[6]
                    if update_quantite_in_classeur(classeur, nom_carte, set_code, val, rarete):
                        new_vals = list(vals)
                        new_vals[3] = val
                        tree.item(iid, values=new_vals)
                qty_win.destroy()
            except ValueError:
                messagebox.showwarning("Erreur", "Veuillez entrer un nombre entier valide")
        ttk.Button(qty_win, text="Valider", command=apply_qty).pack(pady=5)
        entry.bind('<Return>', lambda e: apply_qty())

    # Ajout d'une fonction de refresh qui conserve la sélection multiple et applique les filtres
    def refresh_treeview():
        # Sauvegarde la sélection actuelle (clés logiques)
        selected_keys = []
        for iid in tree.selection():
            vals = tree.item(iid, 'values')
            if vals and len(vals) >= 6:
                key = (vals[0], vals[2], vals[5])  # name, set_code, classeur
                selected_keys.append(key)
        # Récupère les filtres
        filtre_rarete = combo_rarete.get()
        filtre_code = entry_code.get().strip().lower()
        filtre_display = combo_display.get()
        # Recharge les cartes et les valeurs de filtres
        cartes, raretes, displays = get_cartes()
        # Met à jour les listes déroulantes si besoin
        combo_rarete['values'] = ["(Tous)"] + raretes
        combo_display['values'] = ["(Tous)"] + displays
        # Restaure la valeur précédente si elle existe
        if filtre_rarete in combo_rarete['values']:
            combo_rarete.set(filtre_rarete)
        else:
            combo_rarete.set("(Tous)")
        if filtre_display in combo_display['values']:
            combo_display.set(filtre_display)
        else:
            combo_display.set("(Tous)")
        # Efface et recharge les cartes filtrées
        tree.delete(*tree.get_children())
        iid_map = {}
        for carte in cartes:
            # Application des filtres
            if filtre_rarete and filtre_rarete != "(Tous)" and (carte.get("card_sets_set_rarity") or "") != filtre_rarete:
                continue
            if filtre_code and filtre_code not in (carte.get("card_sets_set_code") or "").lower():
                continue
            if filtre_display and filtre_display != "(Tous)" and (carte.get("card_sets_set_name") or "") != filtre_display:
                continue
            values = (
                carte["name"], 
                carte["card_sets_set_name"], 
                carte["card_sets_set_code"], 
                carte["quantite"],
                carte.get("qualite", ""),
                carte["classeur"],
                carte["card_sets_set_rarity"]
            )
            iid = tree.insert("", "end", values=values)
            iid_map[(values[0], values[2], values[5])] = iid
        # Restaure la sélection multiple
        new_selection = [iid_map[key] for key in selected_keys if key in iid_map]
        if new_selection:
            tree.selection_set(new_selection)
            if len(new_selection) == 1:
                tree.focus(new_selection[0])
                tree.see(new_selection[0])

    # Remplissage initial du tableau (treeview)
    refresh_treeview()

    # Lier le bouton "Filtrer" au refresh avec filtres
    btn_filtrer.config(command=refresh_treeview)

    # Lier le bouton "Réinitialiser" pour vider les filtres et rafraîchir
    def reset_filters():
        combo_rarete.set("(Tous)")
        entry_code.delete(0, tk.END)
        combo_display.set("(Tous)")
        refresh_treeview()
    btn_reset.config(command=reset_filters)

    # Ajout : gestion de la sélection multiple par Ctrl+A
    def select_all(event=None):
        tree.selection_set(tree.get_children())
        return "break"
    tree.bind("<Control-a>", select_all)
    tree.bind("<Control-A>", select_all)

    # Ajout : sélection multiple par glissement souris (drag)
    def on_b1_motion(event):
        iid = tree.identify_row(event.y)
        if iid:
            sel = set(tree.selection())
            if iid not in sel:
                tree.selection_add(iid)
    tree.bind('<B1-Motion>', on_b1_motion)

    # Ne pas faire : tree.refresh_inventaire = refresh_treeview
    # Tkinter.Treeview ne supporte pas l'ajout dynamique d'attributs personnalisés.
    # Cela provoque une erreur de type (mypy/pylance) car Treeview n'a pas cet attribut dans sa définition.

    # Solution : retourne la fonction de refresh comme second élément du return
    return main_panel, refresh_treeview

# Ce fichier contient la logique UI :
# - Création et gestion des widgets Tkinter (frames, labels, combobox, treeview, etc.)
# - Gestion des événements utilisateur (bind, callback, édition directe, etc.)
# - Appels aux fonctions du module inventaire_carte (service) pour la logique métier
# - Aucun accès direct à la base de données ou logique métier ici (tout passe par inventaire_carte et update_quantite/qualite)

from tkinter import messagebox
from .controle_version_database_api import check_for_updates, update_database

def check_updates_ui():
    """
    Vérifie les mises à jour disponibles et affiche une notification à l'utilisateur.
    Returns:
        bool: True si une mise à jour a été effectuée, False sinon
    """
    try:
        update_available, version_info = check_for_updates()
        if 'error' in version_info:
            messagebox.showerror("Erreur", f"Impossible de vérifier les mises à jour : {version_info['error']}")
            return False

        if update_available:
            if messagebox.askyesno(
                "Mise à jour disponible", 
                f"Une nouvelle version de la base de données est disponible.\n\n"
                f"Version actuelle: {version_info['local']}\n"
                f"Nouvelle version: {version_info['remote']}\n\n"
                "Voulez-vous mettre à jour maintenant ?"
            ):
                success, message = update_database()
                if success:
                    messagebox.showinfo("Succès", message)
                    return True
                else:
                    messagebox.showerror("Erreur", message)
                    return False
        else:
            messagebox.showinfo("À jour", "Votre base de données est à jour.")
            return False

    except Exception as e:
        messagebox.showerror("Erreur", f"Erreur lors de la vérification des mises à jour : {str(e)}")
        return False

__all__ = ["check_updates_ui"]

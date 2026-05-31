"""
Module centralisé pour les boîtes de dialogue Tkinter.
Tous les modules internes doivent importer depuis ici — jamais depuis main.py.
"""

# Yu-Gi-Oh! Collection Manager
# Copyright (C) 2026  Althalusse
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.

from tkinter import messagebox


def afficher_info(message, titre="Info"):
    messagebox.showinfo(titre, message)


def afficher_warning(message, titre="Attention"):
    messagebox.showwarning(titre, message)

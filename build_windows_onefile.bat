@echo off
setlocal enabledelayedexpansion
chcp 65001 > nul

echo =====================================================
echo   BUILD Yu-Gi-Oh Collection Manager
echo   PyInstaller --onefile
echo =====================================================
echo.

REM -----------------------------
REM Verifications preliminaires
REM -----------------------------
if not exist "main.py" (
    echo [ERREUR] main.py introuvable - lancez ce script depuis la racine du projet.
    pause
    exit /b 1
)

if not exist "requirements.txt" (
    echo [ERREUR] requirements.txt introuvable - lancez ce script depuis la racine du projet.
    pause
    exit /b 1
)

if not exist "module\i18n\fr.json" (
    echo [ERREUR] module\i18n\fr.json introuvable - structure projet incorrecte.
    pause
    exit /b 1
)

if not exist "app_icon.ico" (
    echo [ERREUR] app_icon.ico introuvable a la racine du projet.
    echo          Placez votre icone choisie sous le nom "app_icon.ico" ici, a cote de main.py.
    pause
    exit /b 1
)

REM -----------------------------
REM Creation environnement isole
REM -----------------------------
echo [1/6] Creation de l'environnement virtuel...
if not exist "venv_build" (
    python -m venv venv_build
    if errorlevel 1 (
        echo [ERREUR] Echec creation venv. Verifiez l'installation Python.
        pause
        exit /b 1
    )
)
call venv_build\Scripts\activate.bat
if errorlevel 1 (
    echo [ERREUR] Echec activation venv.
    pause
    exit /b 1
)

REM -----------------------------
REM Upgrade pip
REM -----------------------------
echo.
echo [2/6] Mise a jour de pip...
python -m pip install --upgrade pip

REM -----------------------------
REM Installer dependances projet
REM -----------------------------
echo.
echo [3/6] Installation des dependances du projet...
pip install -r requirements.txt
if errorlevel 1 (
    echo [ERREUR] Echec installation requirements.txt
    pause
    exit /b 1
)

REM PyInstaller
echo.
echo [4/6] Installation de PyInstaller...
pip install pyinstaller
if errorlevel 1 (
    echo [ERREUR] Echec installation PyInstaller
    pause
    exit /b 1
)

REM -----------------------------
REM Nettoyage ancien build
REM -----------------------------
echo.
echo [5/6] Nettoyage des anciens builds...
if exist "build" rmdir /s /q build
if exist "dist"  rmdir /s /q dist
del /q *.spec 2>nul

REM -----------------------------
REM Compilation PyInstaller --onefile
REM
REM Mode --onefile :
REM   Genere un .exe unique et portable (~50-80 Mo). Au lancement, il
REM   s'auto-extrait dans %TEMP%\_MEIxxxxx\ avant d'executer Python.
REM
REM Avantages :
REM   - Un seul fichier a deplacer/distribuer.
REM   - Peut etre pose sur le bureau, une cle USB, n'importe ou.
REM   - Les dossiers de donnees (bdd\, img\, classeurs\) sont crees
REM     a cote du .exe a son emplacement reel (geres par get_exe_dir()
REM     dans centralisation_dossier.py via sys.executable).
REM
REM Inconvenients :
REM   - Demarrage plus lent (5-10 sec : extraction temp a chaque lancement).
REM   - Antivirus parfois suspicieux (faux positif Windows Defender courant
REM     sur les binaires auto-extractibles non signes).
REM   - Plus difficile a debugger (logs perdus dans le dossier temp).
REM
REM Hidden imports :
REM   --hidden-import=PIL.ImageTk    Liaison Tk de Pillow (utilisee par
REM                                  gestion_img/, ui/dialog_*, etc.).
REM   --hidden-import=PIL.Image      Defensive : detecte normalement auto.
REM   --hidden-import=PIL.ImageEnhance Defensive : utilisee pour les
REM                                                cartes desaturees (alts).
REM
REM Assets embarques par les libs tierces :
REM   --collect-data=customtkinter   Themes JSON et police par defaut de CTk.
REM
REM Ressources i18n du projet :
REM   --add-data="module\i18n\fr.json;module\i18n"
REM   --add-data="module\i18n\en.json;module\i18n"
REM     En --onefile, ces JSON sont copies dans %TEMP%\_MEIxxxxx\module\i18n\
REM     a chaque lancement. Le code i18n utilise os.path.dirname(__file__)
REM     qui pointe correctement vers ce dossier au runtime.
REM
REM Fenetre Ko-fi integree (un seul .exe) :
REM   --collect-all=webview   Embarque pywebview + son backend (hook PyInstaller
REM                           officiel : gere pythonnet/clr sur Windows).
REM   --hidden-import=clr     pythonnet (backend EdgeChromium). Defensif :
REM                           simple avertissement si absent, build non bloque.
REM   L'ecran Contribution relance LE .EXE lui-meme avec --kofi : main.py
REM   intercepte ce flag et appelle kofi_viewer.run() dans un process dedie
REM   (sans mainloop Tkinter -> pywebview sur son thread principal). Donc UN
REM   SEUL .exe, et kofi_viewer.py est embarque automatiquement car main.py
REM   l'importe (aucun --add-data necessaire pour lui).
REM   Sans WebView2 / pywebview au runtime -> ouverture navigateur (fallback).
REM -----------------------------

echo.
echo [6/6] Compilation en cours (peut prendre 2-5 minutes)...
echo.

pyinstaller main.py ^
  --onefile ^
  --windowed ^
  --clean ^
  --noconfirm ^
  --name="YGO_Collection_Manager" ^
  --icon="app_icon.ico" ^
  --hidden-import=PIL.ImageTk ^
  --hidden-import=PIL.Image ^
  --hidden-import=PIL.ImageEnhance ^
  --hidden-import=module.donnees.sync_reference ^
  --hidden-import=module.donnees.overframe_enrichment ^
  --collect-data=customtkinter ^
  --collect-all=webview ^
  --hidden-import=clr ^
  --add-data="module\i18n\fr.json;module\i18n" ^
  --add-data="module\i18n\en.json;module\i18n" ^
  --exclude-module=matplotlib ^
  --exclude-module=scipy ^
  --exclude-module=sympy ^
  --exclude-module=numba ^
  --exclude-module=pandas ^
  --exclude-module=PyQt5 ^
  --exclude-module=PyQt6 ^
  --exclude-module=PySide2 ^
  --exclude-module=PySide6 ^
  --exclude-module=wx ^
  --exclude-module=IPython ^
  --exclude-module=jupyter ^
  --exclude-module=notebook ^
  --exclude-module=pytest ^
  --exclude-module=unittest ^
  --exclude-module=doctest

if errorlevel 1 (
    echo.
    echo [ERREUR] Echec compilation PyInstaller. Voir le log ci-dessus.
    pause
    exit /b 1
)

REM -----------------------------
REM Bilan
REM -----------------------------
echo.
echo =====================================================
echo   BUILD --onefile TERMINE AVEC SUCCES
echo =====================================================
echo.
echo Executable unique : dist\YGO_Collection_Manager.exe
echo.
echo Tu peux deplacer ce .exe ou tu veux (bureau, cle USB, Documents...).
echo Au premier lancement, il creera bdd\, img\, classeurs\ et
echo app_config.json a cote de lui.
echo.
echo Premier demarrage : 5-10 sec (extraction temp).
echo Demarrages suivants : 5-10 sec aussi (extraction a chaque fois).
echo.
echo Si Windows Defender bloque le .exe :
echo   - Clic droit sur le .exe ^> Proprietes ^> Debloquer
echo   - Ou ajouter une exception dans Windows Security
echo.
pause
endlocal

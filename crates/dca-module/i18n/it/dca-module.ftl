# Modulo Allineatori (italiano).
# Chiavi del menu: aligners-menu-<percorso>; il percorso è l'id del comando `aligners.<percorso>` con i punti
# sostituiti da trattini (gli id Fluent non possono contenere punti). Descrizioni: aligners-desc-<percorso>.

aligners-name = Allineatori
aligners-select-hint = Seleziona una voce del menu Allineatori.
aligners-status-stub = Non ancora disponibile
aligners-status-stub-detail = Questa funzione è prevista e non è ancora implementata.
aligners-staging-stages = Stadi
aligners-staging-col-stage = Stadio
aligners-staging-col-teeth = Denti in movimento
aligners-staging-col-max-translation = Traslazione massima
aligners-staging-col-max-rotation = Rotazione massima
aligners-staging-col-ipr = Stripping
aligners-staging-no-stages = Nessuno stadio per ora.
aligners-staging-limits = Limiti per allineatore
aligners-staging-step-linear = Traslazione (mm)
aligners-staging-step-vertical = Intrusione / estrusione (mm)
aligners-staging-step-rotation = Rotazione (gradi)
aligners-staging-step-rotation-round = Rotazione, corone tonde (gradi)
aligners-staging-step-angular = Torque e tip (gradi)
aligners-staging-max-crown-step = Limite per un punto della corona (mm)
aligners-staging-interval-days = Giorni per allineatore

aligners-menu-case = Caso
aligners-desc-case = Importa le scansioni del paziente e imposta il caso.

aligners-menu-case-import-scans = Importa scansioni
aligners-desc-case-import-scans = Carica le scansioni delle arcate superiore e inferiore (STL, PLY, OBJ). L'importazione non avvia nessuna elaborazione.

aligners-menu-case-units-orientation = Unità e orientamento
aligners-desc-case-units-orientation = Verifica l'unità di misura delle scansioni (mm, cm, pollici) e l'orientamento del modello nel caso.

aligners-menu-case-case-info = Dati del caso
aligners-desc-case-case-info = Codice paziente pseudonimizzato e note del caso.

aligners-menu-preparation = Preparazione
aligners-desc-preparation = Porta le scansioni in un riferimento comune e individua i denti.

aligners-menu-preparation-orientation = Orientamento automatico e arcata
aligners-desc-preparation-orientation = Arcata superiore o inferiore, piano occlusale e linea mediana: il riferimento su cui lavorano tutte le fasi successive.

aligners-menu-preparation-orientation-automatic = Automatico
aligners-desc-preparation-orientation-automatic = Riconosce automaticamente l'arcata e il suo orientamento.

aligners-menu-preparation-orientation-manual = Manuale: scegli l'arcata
aligners-desc-preparation-orientation-manual = Scegli l'arcata e imposta a mano il piano occlusale e la linea mediana.

aligners-menu-preparation-numbering = Numerazione dei denti (FDI)
aligners-desc-preparation-numbering = Un punto (seme) sulla faccia occlusale di ogni dente con il suo numero FDI: indica al programma quali denti ci sono e dove.

aligners-menu-preparation-numbering-automatic = Trova i denti in automatico
aligners-desc-preparation-numbering-automatic = Individua i denti e li numera automaticamente.

aligners-menu-preparation-numbering-place-seeds = Manuale: posiziona i semi
aligners-desc-preparation-numbering-place-seeds = Posiziona un seme su ogni dente con il mouse e scegli il suo numero FDI.

aligners-menu-preparation-numbering-renumber = Manuale: cambia il numero di un dente
aligners-desc-preparation-numbering-renumber = Cambia il numero FDI di un dente o scambia due denti.

aligners-menu-preparation-numbering-mark-missing = Manuale: segna un dente assente
aligners-desc-preparation-numbering-mark-missing = Segna un dente come assente: il suo spazio resta vuoto.

aligners-menu-preparation-segmentation = Segmentazione
aligners-desc-preparation-segmentation = Ogni punto della scansione viene assegnato a un dente o alla gengiva. Il confine tra i colori diventa il margine della corona.

aligners-menu-preparation-segmentation-automatic = Automatica
aligners-desc-preparation-segmentation-automatic = Separa automaticamente i denti dalla gengiva.

aligners-menu-preparation-segmentation-manual-brush = Pennello manuale
aligners-desc-preparation-segmentation-manual-brush = Dipingi a mano le etichette dei denti con un pennello, alla massima risoluzione.

aligners-menu-preparation-segmentation-clean-borders = Manuale: pulisci i bordi
aligners-desc-preparation-segmentation-clean-borders = Elimina le piccole isole e leviga il confine irregolare tra denti e gengiva.

aligners-menu-preparation-reconstruction = Ricostruzione dei denti
aligners-desc-preparation-reconstruction = Da ogni corona etichettata nasce un dente solido e chiuso, con il suo asse e il suo centro di resistenza.

aligners-menu-preparation-reconstruction-automatic = Automatica
aligners-desc-preparation-reconstruction-automatic = Costruisce automaticamente tutti i denti 3D.

aligners-menu-preparation-reconstruction-rebuild-tooth = Manuale: ricostruisci un dente
aligners-desc-preparation-reconstruction-rebuild-tooth = Mostra la zona della scansione di un dente, la verifica e ricostruisce solo quel dente.

aligners-menu-preparation-gingiva = Gengiva
aligners-desc-preparation-gingiva = Gengiva deformabile collegata ai denti 3D.

aligners-menu-diagnosis = Diagnosi
aligners-desc-diagnosis = Spazio, affollamento e complessità del caso, misurati sui denti 3D.

aligners-menu-diagnosis-space-analysis = Analisi dello spazio
aligners-desc-diagnosis-space-analysis = Spazio disponibile e spazio richiesto in ciascuna arcata, e il conseguente affollamento o diastema.

aligners-menu-diagnosis-little-index = Indice di irregolarità di Little
aligners-desc-diagnosis-little-index = Somma degli spostamenti dei punti di contatto dei sei denti anteriori inferiori o superiori.

aligners-menu-diagnosis-bolton = Analisi di Bolton
aligners-desc-diagnosis-bolton = Rapporto tra le larghezze dei denti superiori e inferiori (anteriore e totale), per individuare discrepanze dimensionali.

aligners-menu-diagnosis-complexity = Punteggio di complessità
aligners-desc-diagnosis-complexity = Un punteggio unico per la difficoltà del caso, calcolato dai valori misurati.

aligners-menu-diagnosis-edit-values = Manuale: correggi i valori
aligners-desc-diagnosis-edit-values = Correggi a mano i valori della diagnosi.

aligners-menu-planning = Pianificazione
aligners-desc-planning = Posizione finale dei denti, movimenti, suddivisione in allineatori e calendario del trattamento.

aligners-menu-planning-setup = Setup (posizione finale)
aligners-desc-planning-setup = Dove devono arrivare i denti, calcolata entro i limiti delle evidenze, con stripping ed espansione.

aligners-menu-planning-setup-automatic = Automatico
aligners-desc-planning-setup-automatic = Calcola automaticamente la posizione finale dei denti.

aligners-menu-planning-setup-arch-line = Manuale: modifica la linea d'arcata
aligners-desc-planning-setup-arch-line = Mostra e modifica la linea d'arcata su cui si allineano i denti, poi ricalcola.

aligners-menu-planning-movements = Movimenti per dente (6 gradi di libertà)
aligners-desc-planning-movements = Sposta e ruota ogni dente a mano nei sei gradi di libertà: traslazione e rotazione attorno al centro di resistenza.

aligners-menu-planning-staging = Staging
aligners-desc-planning-staging = Il movimento viene diviso in allineatori senza superare il movimento massimo per allineatore.

aligners-menu-planning-staging-automatic = Automatico
aligners-desc-planning-staging-automatic = Calcola automaticamente gli allineatori, con uno staging senza collisioni.

aligners-menu-planning-staging-tooth-speed = Manuale: rallenta un dente
aligners-desc-planning-staging-tooth-speed = Riduce la velocità di un dente nello staging, per esempio un dente che non ha seguito il piano.

aligners-menu-planning-ipr = Stripping (IPR)
aligners-desc-planning-ipr = Riduzione interprossimale: quanto smalto rimuovere su ogni contatto e a quale allineatore.

aligners-menu-planning-attachments = Attacchi
aligners-desc-planning-attachments = Posizione e forma degli attacchi che danno all'allineatore la presa sui denti.

aligners-menu-planning-schedule = Calendario
aligners-desc-planning-schedule = Giorni per allineatore, ore di utilizzo giornaliere e consegna degli allineatori a lotti.

aligners-menu-planning-clinical-review = Revisione clinica
aligners-desc-planning-clinical-review = Le classificazioni proposte dal programma, da confermare o modificare dal clinico prima della produzione.

aligners-menu-occlusion = Occlusione
aligners-desc-occlusion = Relazione tra l'arcata superiore e l'arcata inferiore.

aligners-menu-occlusion-virtual-bite = Morso virtuale
aligners-desc-occlusion-virtual-bite = Stima l'occlusione delle due arcate quando non c'è una scansione del morso.

aligners-menu-occlusion-bite-registration = Registrazione del morso (ICP)
aligners-desc-occlusion-bite-registration = Relazione tra le arcate ricavata dalle scansioni del morso.

aligners-menu-occlusion-bite-registration-icp = Automatica (ICP)
aligners-desc-occlusion-bite-registration-icp = Registra automaticamente le scansioni del morso sulle arcate.

aligners-menu-occlusion-bite-registration-three-points = Manuale: allineamento a 3 punti
aligners-desc-occlusion-bite-registration-three-points = Allinea le scansioni del morso sulle arcate scegliendo tre punti corrispondenti.

aligners-menu-occlusion-contact-map = Mappa dei contatti
aligners-desc-occlusion-contact-map = Mappa a colori della distanza tra le arcate, per vedere i contatti occlusali.

aligners-menu-occlusion-bite-with-aligners = Morso con gli allineatori
aligners-desc-occlusion-bite-with-aligners = Occlusione con gli allineatori indossati: spessore occlusale e contatti.

aligners-menu-occlusion-bite-ramps = Rampe di morso
aligners-desc-occlusion-bite-ramps = Rampe sugli allineatori che aprono il morso, per esempio per muovere gli incisivi.

aligners-menu-production = Produzione degli allineatori
aligners-desc-production = Bordo, spessore e file degli allineatori per la stampante a resina.

aligners-menu-production-base-labels = Base del modello ed etichette
aligners-desc-production-base-labels = Chiude il modello con una base e aggiunge le etichette utili al tecnico.

aligners-menu-production-shell = Guscio dell'allineatore (spessore e gioco)
aligners-desc-production-shell = Il guscio dell'allineatore attorno ai denti: spessore e gioco interno.

aligners-menu-production-shell-automatic = Automatico
aligners-desc-production-shell-automatic = Costruisce il guscio dell'allineatore per ogni stadio.

aligners-menu-production-shell-corrections = Manuale: correzioni
aligners-desc-production-shell-corrections = Corregge a mano la forma di un allineatore.

aligners-menu-production-trim-line = Linea di rifilatura
aligners-desc-production-trim-line = Bordo dell'allineatore: lungo l'equatore del dente o il margine gengivale, con festoni.

aligners-menu-production-stamp = Timbro identificativo
aligners-desc-production-stamp = Codice del caso e data incisi sull'allineatore.

aligners-menu-production-print-check = Controllo di stampa
aligners-desc-production-print-check = Verifica che gli allineatori siano stampabili: superfici chiuse, parete minima, sottosquadri.

aligners-menu-followup = Monitoraggio
aligners-desc-followup = Confronta il trattamento con il piano durante le visite di controllo.

aligners-menu-followup-visits = Visite di controllo
aligners-desc-followup-visits = Confronta una nuova scansione con il piano: denti in pari o in ritardo.

aligners-menu-followup-tracking = Monitoraggio e ripianificazione
aligners-desc-followup-tracking = Nuovo piano a partire dagli allineatori già consegnati.

aligners-menu-output = Esportazione
aligners-desc-output = File e viste del risultato.

aligners-menu-output-export-stl = Esporta STL per stadio
aligners-desc-output-export-stl = Esporta i modelli e gli allineatori di ogni stadio come file STL.

aligners-menu-output-report = Report
aligners-desc-output-report = Relazione clinica e tecnica del caso.

aligners-menu-output-viewer = Visualizzatore paziente e animazione
aligners-desc-output-viewer = Animazione del trattamento allineatore per allineatore, da mostrare al paziente.

aligners-menu-tools = Strumenti
aligners-desc-tools = Strumenti manuali e cronologia delle elaborazioni.

aligners-menu-tools-manual-tools = Strumenti manuali
aligners-desc-tools-manual-tools = Alternative manuali: ogni fase automatica può essere eseguita o corretta a mano.

aligners-menu-tools-history = Cronologia
aligners-desc-tools-history = Fasi fatte, da fare e da rifare; torna a una fase precedente e annulla il ritorno indietro.

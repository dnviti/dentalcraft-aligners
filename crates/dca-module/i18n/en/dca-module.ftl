# Aligners module (English, base language).
# Menu keys: aligners-menu-<path>; the path is the command id `aligners.<path>` with the dots
# replaced by hyphens (Fluent ids cannot contain dots). Descriptions: aligners-desc-<path>.

aligners-name = Aligners
aligners-select-hint = Select an item of the Aligners menu.
aligners-status-stub = Not yet available
aligners-status-stub-detail = This function is planned and is not implemented yet.
aligners-staging-stages = Stages
aligners-staging-col-stage = Stage
aligners-staging-col-teeth = Teeth moving
aligners-staging-col-max-translation = Max translation
aligners-staging-col-max-rotation = Max rotation
aligners-staging-col-ipr = IPR
aligners-staging-no-stages = No stages yet.
aligners-staging-limits = Limits per aligner
aligners-staging-step-linear = Translation (mm)
aligners-staging-step-vertical = Intrusion / extrusion (mm)
aligners-staging-step-rotation = Rotation (deg)
aligners-staging-step-rotation-round = Rotation, round crowns (deg)
aligners-staging-step-angular = Torque and tip (deg)
aligners-staging-max-crown-step = Crown point cap (mm)
aligners-staging-interval-days = Days per aligner

aligners-menu-case = Case
aligners-desc-case = Import the patient's scans and set up the case.

aligners-menu-case-import-scans = Import scans
aligners-desc-case-import-scans = Load the upper and lower arch scans (STL, PLY, OBJ). Importing never starts any processing.

aligners-menu-case-units-orientation = Units & orientation
aligners-desc-case-units-orientation = Check the unit of the scans (mm, cm, inch) and the orientation of the model in the case.

aligners-menu-case-case-info = Case info
aligners-desc-case-case-info = Pseudonymised patient code and notes of the case.

aligners-menu-preparation = Preparation
aligners-desc-preparation = Bring the scans to a common reference and find the teeth.

aligners-menu-preparation-orientation = Auto orientation & arch
aligners-desc-preparation-orientation = Upper or lower arch, occlusal plane and midline: the reference all later steps work on.

aligners-menu-preparation-orientation-automatic = Automatic
aligners-desc-preparation-orientation-automatic = Recognise the arch and its orientation automatically.

aligners-menu-preparation-orientation-manual = Manual: choose the arch
aligners-desc-preparation-orientation-manual = Choose the arch and set the occlusal plane and the midline by hand.

aligners-menu-preparation-numbering = Tooth numbering (FDI)
aligners-desc-preparation-numbering = A seed point on the occlusal surface of each tooth with its FDI number: it tells the program which teeth exist and where.

aligners-menu-preparation-numbering-automatic = Find teeth automatically
aligners-desc-preparation-numbering-automatic = Find the teeth and number them automatically.

aligners-menu-preparation-numbering-place-seeds = Manual: place seeds
aligners-desc-preparation-numbering-place-seeds = Place a seed on each tooth with the mouse and choose its FDI number.

aligners-menu-preparation-numbering-renumber = Manual: renumber a tooth
aligners-desc-preparation-numbering-renumber = Change the FDI number of a tooth or swap two teeth.

aligners-menu-preparation-numbering-mark-missing = Manual: mark a missing tooth
aligners-desc-preparation-numbering-mark-missing = Mark a tooth as missing: its space stays empty.

aligners-menu-preparation-segmentation = Segmentation
aligners-desc-preparation-segmentation = Every point of the scan is assigned to a tooth or to the gingiva. The border between the colours becomes the margin of the crown.

aligners-menu-preparation-segmentation-automatic = Automatic
aligners-desc-preparation-segmentation-automatic = Separate the teeth from the gingiva automatically.

aligners-menu-preparation-segmentation-manual-brush = Manual brush
aligners-desc-preparation-segmentation-manual-brush = Paint the tooth labels by hand with a brush, at full resolution.

aligners-menu-preparation-segmentation-clean-borders = Manual: clean borders
aligners-desc-preparation-segmentation-clean-borders = Remove small islands and smooth the jagged border between teeth and gingiva.

aligners-menu-preparation-reconstruction = Tooth reconstruction
aligners-desc-preparation-reconstruction = From each labelled crown a solid, closed tooth is built, with its axis and its centre of resistance.

aligners-menu-preparation-reconstruction-automatic = Automatic
aligners-desc-preparation-reconstruction-automatic = Build all the 3D teeth automatically.

aligners-menu-preparation-reconstruction-rebuild-tooth = Manual: rebuild one tooth
aligners-desc-preparation-reconstruction-rebuild-tooth = Show the scan region of a tooth, check it and rebuild only that tooth.

aligners-menu-preparation-gingiva = Gingiva
aligners-desc-preparation-gingiva = Deformable gingiva attached to the 3D teeth.

aligners-menu-diagnosis = Diagnosis
aligners-desc-diagnosis = Space, crowding and complexity of the case, measured on the 3D teeth.

aligners-menu-diagnosis-space-analysis = Space analysis
aligners-desc-diagnosis-space-analysis = Space available and space required in each arch, and the resulting crowding or spacing.

aligners-menu-diagnosis-little-index = Little irregularity index
aligners-desc-diagnosis-little-index = Sum of the displacements of the contact points of the six lower or upper anterior teeth.

aligners-menu-diagnosis-bolton = Bolton analysis
aligners-desc-diagnosis-bolton = Ratio between the widths of the upper and lower teeth (anterior and overall), to find tooth-size discrepancies.

aligners-menu-diagnosis-complexity = Complexity score
aligners-desc-diagnosis-complexity = A single score for the difficulty of the case, from the measured values.

aligners-menu-diagnosis-edit-values = Manual: correct the values
aligners-desc-diagnosis-edit-values = Correct the diagnosis values by hand.

aligners-menu-planning = Planning
aligners-desc-planning = Final position of the teeth, movements, division into aligners and treatment schedule.

aligners-menu-planning-setup = Setup (final position)
aligners-desc-planning-setup = Where the teeth must end up, computed within the limits of the evidence, with IPR and expansion.

aligners-menu-planning-setup-automatic = Automatic
aligners-desc-planning-setup-automatic = Compute the final position of the teeth automatically.

aligners-menu-planning-setup-arch-line = Manual: edit the arch line
aligners-desc-planning-setup-arch-line = Show and edit the arch line the teeth are aligned to, then recompute.

aligners-menu-planning-movements = Per-tooth movements (6-DOF)
aligners-desc-planning-movements = Move and rotate each tooth by hand in all six degrees of freedom: translation and rotation about the centre of resistance.

aligners-menu-planning-staging = Staging
aligners-desc-planning-staging = The movement is divided into aligners without exceeding the maximum movement per aligner.

aligners-menu-planning-staging-automatic = Automatic
aligners-desc-planning-staging-automatic = Compute the aligners automatically with collision-free staging.

aligners-menu-planning-staging-tooth-speed = Manual: slow down a tooth
aligners-desc-planning-staging-tooth-speed = Reduce the speed of one tooth in the staging, for example a tooth that did not track.

aligners-menu-planning-ipr = IPR
aligners-desc-planning-ipr = Interproximal reduction: how much enamel to remove on each contact, and at which aligner.

aligners-menu-planning-attachments = Attachments
aligners-desc-planning-attachments = Position and shape of the attachments that give the aligner grip on the teeth.

aligners-menu-planning-schedule = Schedule
aligners-desc-planning-schedule = Days per aligner, daily wear and delivery of the aligners in batches.

aligners-menu-planning-clinical-review = Clinical review
aligners-desc-planning-clinical-review = The classifications proposed by the program, to be confirmed or changed by the clinician before production.

aligners-menu-occlusion = Occlusion
aligners-desc-occlusion = Relation between the upper and the lower arch.

aligners-menu-occlusion-virtual-bite = Virtual bite
aligners-desc-occlusion-virtual-bite = Estimate the occlusion of the two arches when no bite scan is available.

aligners-menu-occlusion-bite-registration = Bite registration (ICP)
aligners-desc-occlusion-bite-registration = Relation between the arches from the bite scans.

aligners-menu-occlusion-bite-registration-icp = Automatic (ICP)
aligners-desc-occlusion-bite-registration-icp = Register the bite scans on the arches automatically.

aligners-menu-occlusion-bite-registration-three-points = Manual: 3-point alignment
aligners-desc-occlusion-bite-registration-three-points = Align the bite scans on the arches by picking three matching points.

aligners-menu-occlusion-contact-map = Contact map
aligners-desc-occlusion-contact-map = Colour map of the distance between the arches, to see the occlusal contacts.

aligners-menu-occlusion-bite-with-aligners = Bite with aligners
aligners-desc-occlusion-bite-with-aligners = Occlusion with the aligners worn: occlusal thickness and contacts.

aligners-menu-occlusion-bite-ramps = Bite ramps
aligners-desc-occlusion-bite-ramps = Ramps on the aligners that open the bite, for example to move the incisors.

aligners-menu-production = Aligner production
aligners-desc-production = Edge, thickness and files of the aligners for the resin printer.

aligners-menu-production-base-labels = Model base & labels
aligners-desc-production-base-labels = Close the model with a base and add the labels the technician needs.

aligners-menu-production-shell = Aligner shell (thickness & clearance)
aligners-desc-production-shell = The aligner shell around the teeth: thickness and internal clearance.

aligners-menu-production-shell-automatic = Automatic
aligners-desc-production-shell-automatic = Build the shell of the aligner for each stage.

aligners-menu-production-shell-corrections = Manual: corrections
aligners-desc-production-shell-corrections = Correct the shape of one aligner by hand.

aligners-menu-production-trim-line = Trim line
aligners-desc-production-trim-line = Edge of the aligner: along the tooth equator or the gingival margin, with scallops.

aligners-menu-production-stamp = Identification stamp
aligners-desc-production-stamp = Case code and date engraved on the aligner.

aligners-menu-production-print-check = Print check
aligners-desc-production-print-check = Checks that the aligners can be printed: closed surfaces, minimum wall, undercuts.

aligners-menu-followup = Follow-up
aligners-desc-followup = Compare the treatment with the plan at the control visits.

aligners-menu-followup-visits = Control visits
aligners-desc-followup-visits = Compare a new scan with the plan: teeth on track or behind.

aligners-menu-followup-tracking = Tracking & re-plan
aligners-desc-followup-tracking = New plan starting from the aligners already delivered.

aligners-menu-output = Output
aligners-desc-output = Files and views of the result.

aligners-menu-output-export-stl = Export STL per stage
aligners-desc-output-export-stl = Export the models and the aligners of each stage as STL files.

aligners-menu-output-report = Report
aligners-desc-output-report = Clinical and technical report of the case.

aligners-menu-output-viewer = Patient viewer & animation
aligners-desc-output-viewer = Animation of the treatment aligner by aligner, to show to the patient.

aligners-menu-tools = Tools
aligners-desc-tools = Manual tools and history of the processing.

aligners-menu-tools-manual-tools = Manual tools
aligners-desc-tools-manual-tools = Manual fallbacks: every automatic step can be done or corrected by hand.

aligners-menu-tools-history = History
aligners-desc-tools-history = Steps done, to do and to redo; go back to an earlier step and undo going back.

## Commands: label and documentation of every registered command (CommandSpec), one
## section per stage; append the keys of a stage under its own header only.
## Keys: aligners-cmd-<stage>-<name> (label) and aligners-cmd-<stage>-<name>-doc (what it does
## and what the parameters mean). A command that is also a menu leaf may use its
## aligners-menu-<path> key as label.

## ---- commands: frame ----

aligners-cmd-frame-world = World frame
aligners-cmd-frame-world-doc = Puts the patient on the world axes: computes the rigid transform that moves the case so that the origin is the centre of the dental arch on the occlusal plane, the midline runs along Y with the incisors towards -Y and the patient's right towards -X, the lower arch has the teeth up (+Z) and the upper arch the teeth down. The scans are not modified: the transform is stored in the case. The reference is the lower arch, or the upper arch when there is no lower one. The arch frame comes from the seeds of the reference arch when there are enough of them, otherwise it is estimated from the shape of the scan. Parameter: reference (auto, upper or lower).

## ---- commands: demo ----

aligners-cmd-demo-generate = Demo arch
aligners-cmd-demo-generate-doc = Generates a synthetic dental arch (a typodont, not patient data) with the ground truth: the scan is added to the case as the upper or lower scan, replacing the one already there, and the seed of every tooth and the per-vertex tooth labels are stored in the Aligners data. The same parameters always give the same model. Parameters: arch (upper or lower), severity (mild, moderate or severe: how much the teeth are rotated, displaced and crowded), seed (number of the random draw), third_molars (include the third molars, default false).

## ---- commands: autoseed ----

aligners-cmd-autoseed-orientation-doc = Puts the patient on the world axes, like the world frame command, and stores the transform in the case (the scans are not modified): the origin is the centre of the dental arch on the occlusal plane, the incisors towards -Y, the patient's right towards -X, the lower arch with the teeth up and the upper arch with the teeth down. The arch frame comes from the seeds of the reference arch when there are enough of them, otherwise from the crowns of the scan (occlusal direction from the open border or the flat base, anterior direction from the U of the crown tops). Parameter: reference (auto, upper or lower; auto is the lower arch, else the upper).
aligners-cmd-autoseed-numbering-doc = Finds the teeth of each arch scan and puts a numbered seed (FDI) on the occlusal face of every tooth, replacing the seeds of that arch. The crown profile along the arch is matched to the typical tooth widths for every plausible number of molars; a tooth inside the arch may be missing. The proposal must be checked. It fails when fewer than 4 teeth are found: place the seeds manually. Parameters: arch (all, upper or lower), third_molars (third molars may be present, default false), missing (FDI numbers of the teeth known to be missing), ends (forced distal tooth of the right and left side, positions 5 to 8, e.g. [7, 7]; default chosen by the data).

## ---- commands: segmentation ----

## ---- commands: teeth ----
aligners-cmd-teeth-frames = Tooth frames and widths
aligners-cmd-teeth-frames-doc = Measures every segmented tooth: the local frame (mesio-distal, bucco-lingual and long axis, in case coordinates), the mesio-distal and bucco-lingual widths, the crown height, the cusp tip, the interproximal contacts and the estimated cemento-enamel junction, alveolar crest, root apex and centre of resistance (population estimates, a surface scan has no roots). The long axis comes from the gingival margin and the side walls of the crown, the mesio-distal axis from the contacts, the incisal edge or the symmetry of the crown. The frames are stored in the case and the measures are returned; the scans are not modified. Needs the segmentation. Parameters: arch (all, upper or lower), margin_to_crest_mm, cres_ratio, max_tilt_deg.

## ---- commands: reconstruct ----

## ---- commands: gingiva ----

## ---- commands: diagnosis ----

## ---- commands: setup ----

## ---- commands: staging ----

## ---- commands: schedule ----

## ---- commands: occlusion ----

## ---- commands: bite ----

## ---- commands: ramps ----

## ---- commands: shell ----

## ---- commands: trim ----

## ---- commands: stamp ----

## ---- commands: export ----

## ---- commands: visits ----

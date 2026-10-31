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

aligners-menu-output-print-check = Print check
aligners-desc-output-print-check = Checks that each model and aligner is ready to print: closed, manifold, no self-intersections, minimum wall, fits the build plate.

aligners-menu-output-manifest = Manifest
aligners-desc-output-manifest = BLAKE3 hashes and sizes of the files of an export, and their verification.

aligners-menu-output-report = Report
aligners-desc-output-report = Clinical and technical report of the case, written with the print files, the patient programmes and the manifest of the export.

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
aligners-cmd-segmentation-automatic = Automatic segmentation
aligners-cmd-segmentation-automatic-doc = Assigns every vertex of the scan to a tooth (FDI number) or to the gingiva (0), starting from the tooth seeds. The scan is moved to the arch frame of its seeds and simplified to about 30 000 faces; a random walk on its face graph, guided by the folds of the surface (Lai 2009) and seeded by a disk around every tooth seed and by gingival constraints (Zou 2015, Liao 2015), gives the first partition, repeated with every seed moved to the centre of its crown. The labels are transferred to the full scan, cleaned (one connected piece per tooth) and their boundaries refined at full resolution. The labels are stored in the case and replace the previous labels of the arch; the scans and the seeds are not modified. Needs the seeds. Parameters: arch (upper or lower, default all arches with seeds), target_faces, eta_convex, sigma, refine.

## ---- commands: teeth ----
aligners-cmd-teeth-frames = Tooth frames and widths
aligners-cmd-teeth-frames-doc = Measures every segmented tooth: the local frame (mesio-distal, bucco-lingual and long axis, in case coordinates), the mesio-distal and bucco-lingual widths, the crown height, the cusp tip, the interproximal contacts and the estimated cemento-enamel junction, alveolar crest, root apex and centre of resistance (population estimates, a surface scan has no roots). The long axis comes from the gingival margin and the side walls of the crown, the mesio-distal axis from the contacts, the incisal edge or the symmetry of the crown. The frames are stored in the case and the measures are returned; the scans are not modified. Needs the segmentation. Parameters: arch (all, upper or lower), margin_to_crest_mm, cres_ratio, max_tilt_deg.

## ---- commands: reconstruct ----

aligners-cmd-reconstruction-automatic-doc = Closes every segmented tooth into a watertight solid. The teeth are measured first (tooth frames, as Tooth frames and widths does); each crown is then cut along a smooth margin line, the surface the scanner closed over the contacts, thin sheets and the strip next to the margin are rebuilt, and the opening is closed by a fair surface tangent to the crown that narrows to a neck below the gingival margin and ends in a rounded stump (thin-plate fairing, Kobbelt 1998). The rebuilt surface follows the convex outline of every horizontal section of the crown and reaches the contact surface where the scan joined two teeth. The solids are stored in the case (case coordinates) and their numbers returned; the scans are not modified. Needs the segmentation. Parameters: arch (all, upper or lower), neck_depth_mm, apex_mm, neck_scale.
aligners-cmd-reconstruction-rebuild-tooth-doc = Closes one tooth again into a watertight solid, with its stored frame (measured first when missing), and replaces its solid in the case. Needs the segmentation. Parameters: fdi (the tooth number, required), neck_depth_mm, apex_mm, neck_scale.

## ---- commands: gingiva ----

aligners-cmd-gingiva-doc = Builds the gingiva without holes of every segmented arch: the scan cut along the smoothed gingival margin of all the teeth, continued under the crowns by a harmonic membrane lowered up to col_mm (the ridge under the teeth and the interdental col seen when teeth move apart). It is stored in the case with the tooth every vertex moves with; harmonic blend weights then move it with the teeth (margin rigid with its tooth, papillae shared by neighbouring teeth, base still). The scans are not modified. Needs the segmentation. Parameters: arch (all, upper or lower), col_mm, papilla_mm, collar_mm, collar_gap_mm, radius_mm, root_mm, root_fade_mm.

## ---- commands: diagnosis ----
aligners-cmd-diagnosis-space-analysis-doc = Runs the whole diagnosis of the segmented arches from the measured teeth and stores it in the case: the space available between the mesial contacts of the first molars (arc length on a fourth-order polynomial fitted to the teeth, AlHarbi 2008) against the space required (sum of the mesio-distal widths), the Little irregularity index, the largest space between crowns, the Bolton ratios and the CAT-CAT complexity score. Returns the report; negative discrepancy means crowding. Needs the segmentation and the teeth frames. Parameters: arch (all, upper or lower).
aligners-cmd-diagnosis-little-index-doc = Runs the whole diagnosis (see the space analysis) and returns it with the Little irregularity index (Little 1975): the sum of the five displacements, in the occlusal plane, between the anatomical contact points of the six anterior teeth, with its category. Needs the segmentation and the teeth frames. Parameters: arch (all, upper or lower).
aligners-cmd-diagnosis-bolton-doc = Runs the whole diagnosis (see the space analysis) and returns it with the Bolton ratios (Bolton 1958): lower over upper mesio-distal widths, anterior (canine to canine, norm 77.2 +- 1.65 %) and overall (first molar to first molar, norm 91.3 +- 1.91 %). Needs both arches to be measured. Parameters: arch (all, upper or lower).
aligners-cmd-diagnosis-complexity-doc = Runs the whole diagnosis (see the space analysis) and returns it with the complexity score: the CAT-CAT model-analysis items that the scan gives (crowding, rotations of premolars and lateral incisors, midline deviation, largest diastema; Long 2020) and the difficulty level of the 2025 consensus (Wang 2025). Needs the segmentation and the teeth frames. Parameters: arch (all, upper or lower).

## ---- commands: setup ----
aligners-cmd-setup-automatic-doc = Computes the final position of the teeth (the setup): every tooth to align ends with its prominent cusps (incisal edge, canine tip, buccal cusps) on the patient's own arch line, the cusps smoothed by a robust regression, or on the line drawn by the clinician. A tooth moves only when it is off the line, rotated or stepped beyond the tolerances of its zone; it is derotated about its long axis until it is tangent to the line and levelled only beyond the tolerated step, tip and torque unchanged. The space missing between the fixed teeth (molars, teeth left out of the aligner, locked teeth) is made first by the therapy in the chosen order and only as much as needed: IPR within the limits per surface, expansion, incisor advancement; the crowding left is spread over the contacts and reported, with close_spaces the gaps close. Locked teeth keep their final position and are the reference. The final poses and the IPR plan are stored in the case and the summary is returned (moved teeth, IPR per contact, expansion, advancement, residual crowding, notes). Needs the segmentation. Parameters: arch (all, upper or lower), align_scope, priority, ipr_zone, leveling, midline, close_spaces, excluded (teeth out of the aligner), line (drawn arch line, case coordinates, one arch), tolerances and limits (line_sigma_mm, pos_tol_mm, normal_tol_mm, rot_tol_deg, post_normal_tol_mm, post_rot_tol_deg, level_tol_mm, vertical_max_mm, expansion_upper_mm, expansion_lower_posterior_mm, expansion_lower_canine_mm, advance_max_mm, retract_max_mm, ipr_surface_cap_mm, enamel_fraction).

## ---- commands: staging ----
aligners-cmd-staging-automatic-doc = Divides the movement of every tooth, from its rest position to the setup, into aligners. The number of aligners of a tooth is the smallest one that keeps every component under its limit per aligner (translation 0.25 mm, torque and tip 1 degree, rotation 2 degrees or 1.5 for canines and premolars) and every crown point under the crown cap (0.33 mm); a periodontal stage reduces the limits. The pattern (proportional, maximum velocity, sequential distalization, structured or automatic from the complexity of the case) decides when each tooth moves. Adjacent crowns are checked at every aligner and the tooth that moves into its neighbour is delayed; the aligner at which each IPR is done is returned. The aligners are stored in the case and the notes, the collisions that remain and the overlap of the final setup are returned. Needs the tooth frames and the setup. Parameters: arch (all, upper or lower), pattern, perio, collision, collision_tol_mm, overcorrection, step_linear_mm, step_vertical_mm, step_rotation_deg, step_rotation_round_deg, step_angular_deg, max_crown_step_mm (what is not given comes from the settings of the case).

## ---- commands: schedule ----
aligners-cmd-schedule-doc = Builds the change calendar of the aligners under full compliance (at least 22 hours a day, changed on the due date): first and last day of every aligner, days of wear (the same for all, or 7 days for anterior translations and 14 days when the aligner has posterior angular or vertical movements), what to do before it (attachments, IPR) and the passive aligners that keep the two arches changing together. The calendar is returned and the case is not modified. Needs the staging. Parameters: arch (all, upper or lower), start_date (YYYY-MM-DD, default today), interval_days, policy (fixed or adaptive), lang (en or it), attachments (teeth bonded before aligner 1).

## ---- commands: occlusion ----
aligners-cmd-occlusion-virtual-bite-doc = Puts the upper arch in occlusion on the lower one when there is no bite scan: an estimated occlusion for display, not the relation registered on the patient. The upper arch is turned to face the lower one with the midlines aligned, the upper incisors in front of the lower ones by the overjet and the upper first molars distal to the lower ones (Class I); small turns and shifts around that pose are lowered to the first contact and the one with the most contacts on both sides is kept, then small tilts let it rest on both sides and the first contact is measured exactly on the crowns. The upper scan, its labels, seeds and tooth frames move rigidly; the lower arch stays in place. Needs both arches segmented with their seeds. Parameters: search (search the maximum intercuspation, default true), overjet_mm, class_i_mm.
aligners-cmd-occlusion-contact-map-doc = Measures the occlusal contacts of the two arches as they are in the case: the signed distance of every crown vertex to the crowns of the other arch (negative inside a crown, up to 0.5 mm), the teeth in contact (within 0.10 mm) with their smallest distance, the teeth in contact on each side and the deepest interpenetration. The case is not modified; the app colours the map on the teeth. Needs both arches segmented with their seeds.

## ---- commands: bite ----
aligners-cmd-bite-icp-doc = Registers the two arches to the buccal bite scans of the case (one or two scans with the role bite, taken with the patient biting): each bite scan is found on the buccal surface of the lower arch by a coarse search and a trimmed point-to-plane ICP on the crowns, its points on the lower arch are removed and the upper arch is registered to the rest. The lower arch stays in place; the bite scans move onto it and the upper scan, its labels, seeds and tooth frames move onto them. Returns the fit of every scan (overlap area, teeth, RMS, coherence), the agreement of two scans, the interpenetration and the verdict with the reasons to review it. Nothing is moved when the registration is rejected (too little overlap, surfaces that do not match, arches that do not face each other or interpenetrate too much). Needs both arches segmented with their seeds.
aligners-cmd-bite-three-points-doc = Registers the two arches to the bite scans like the automatic registration, but starts the first bite scan from points picked by hand: at least 3 points on the lower arch and the matching points on the bite scan (far apart, for example canine, premolar and molar), optionally also on the upper arch, all in case coordinates. Use it when the automatic search fails. Parameters: lower_points, bite_points, upper_points, bite_points_upper (lists of [x, y, z] in mm).

## ---- commands: ramps ----
aligners-cmd-bite-with-aligners-doc = Closes the mouth on the two aligners of a stage and measures the bite with them worn. The outer surface of an aligner is the offset of the model by the clearance and the wall thickness, seen from the occlusal side; the occlusal thickness (mode auto, manual or same) makes the surfaces that chew thinner than the walls, in auto the same on both aligners, the thickest that brings the opening of the bite at the first molars to opening_target_mm, never under min_thickness_mm. The upper arch is moved from the relation of the scans to the first contact of the aligners, by a rotation about the hinge axis of the condyles (closing hinge, hinge_posterior_mm behind and hinge_height_mm above the lower incisal point) or along the occlusal axis (vertical); the incisors and canines are measured in 3D, the other teeth on height maps. The ramps of the case are part of the aligners when they are enabled. With equilibrate the material of both aligners where they would interpenetrate is ground off (never under the minimum, at most equilibrate_max_mm) until equilibrate_teeth_per_side teeth touch on each side. Returns the thickness, the opening at molars and incisors caused by the aligners, the teeth in contact on each side and a sentence (Italian). Needs both arches segmented with their seeds; the arch with fewer aligners stays at its last. The settings used are kept in the case unless save is false. Parameters: stage, mode, thickness_mm, min_thickness_mm, opening_target_mm, closing, hinge_posterior_mm, hinge_height_mm, equilibrate, equilibrate_teeth_per_side, equilibrate_max_mm, save.
aligners-cmd-bite-ramps-doc = Bite ramps: small bumps on the outer surface of an aligner that the antagonist rests on (anterior ramps on the palatal face of the upper incisors open the bite at the back in a deep bite). Action propose (the default): when the overbite of the incisors at the stage is beyond overbite_min_mm, a ramp on the palatal face of every upper central and lateral incisor under the aligner that the lower incisors reach, its top turned half way between the face and the occlusal plane, of the size width_mm x depth_mm x height_mm; it replaces the ramps proposed before and keeps the ones placed by hand, and turns the ramps on. Action add: a ramp by hand on tooth at point (case coordinates of the tooth at rest) with the outward normal. Action remove: the ramps of tooth, all of them without it. Action set: only the settings (enabled, sizes, overbite_min_mm). The ramps follow their tooth at every stage; the shell unites them with the aligner and the bite with the aligners counts them. Needs both arches segmented with their seeds. Parameters: action, stage, enabled, tooth, point, normal, width_mm, depth_mm, height_mm, overbite_min_mm.

## ---- commands: shell ----
aligners-cmd-shell-automatic-doc = Builds the aligner shell of every stage before the trim: the model of the stage (the closed teeth at their stage position and the gingiva moved with them, closed down to a flat base like a trimmed plaster cast) with the crevices narrower than twice blockout_mm filled, then the material between its offsets at the clearance and at the clearance plus the wall thickness (a sheet of uniform thickness laid on the model, built on signed distance fields). Every shell is one closed, manifold surface reduced within the mesh tolerance. The shells are checked and their report returned (triangles, closed, wall thickness measured, seconds); nothing is stored in the case, the export builds them again. Needs the closed teeth and the gingiva. Parameters: arch (all, upper or lower), stages (aligner numbers, default all), thickness_mm and clearance_mm (default the case settings), model_voxel_mm, shell_voxel_mm, blockout_mm, base_height_mm, tolerance_mm, insertion_axis.
aligners-cmd-shell-model-doc = Builds the printable model of every stage: the closed teeth at their stage position and the gingiva moved with them, closed down to a flat base base_height_mm below the gingiva like a trimmed plaster cast (the exact gingiva near the teeth, a height field seen from the occlusal side farther away, never rising away from the teeth), with the crevices narrower than twice blockout_mm filled. Every model is one closed, manifold surface reduced within the mesh tolerance; the labels are not added yet. The models are checked and their report returned; nothing is stored in the case, the export builds them again. Needs the closed teeth and the gingiva. Parameters: as the aligner shell.
aligners-cmd-shell-blockout = Blockout model
aligners-cmd-shell-blockout-doc = Builds the blockout model of every stage: the printable model with every undercut relative to the insertion axis filled (everything hidden from the insertion direction, so that a line along the axis meets the solid once). Every blockout model is one closed, manifold surface reduced within the mesh tolerance. The models are checked and their report returned; nothing is stored in the case. Needs the closed teeth and the gingiva. Parameters: as the aligner shell; insertion_axis is the insertion direction in arch coordinates (default [0, 0, 1], occlusal).

## ---- commands: trim ----
aligners-cmd-trim-line-doc = Computes the trim line of the aligner of every arch and cuts the shell along it. The line follows the mode: equator (the height of contour of every crown along its insertion path, insertion_axis: vertical, common (one rigid path tilted as on a surveyor) or per_tooth (the default, every tooth its own path), moved by equator_offset_mm towards the gingiva, its narrow tabs rounded, never under the gingival margin nor within 1 mm of the top: no undercut and no gingiva covered), straight (the gingival zenith of the buccal and of the lingual side of every tooth, extension_mm below it), scalloped (the gingival margin, extension_mm below it) or smooth (the margin of the covered teeth along the arch, smoothed with gentle scallops of scallop_mm, margin_gap_mm above it, round the distal face of the last tooth of each side; scalloped when fewer than two teeth have a margin). The lines are keyed by tooth; the smooth border is one line of the arch (arch). A trim surface goes through the line, and the shell (a sheet of the wall thickness laid on the model with the crevices narrower than twice blockout_mm filled) keeps what is above it and within reach of the crowns, with a rim rounded by round_mm (default 0.3 x the wall, at most 0.3 mm): one closed, manifold solid. Until the model of a stage comes from the shell stage, the model is the segmented scan, which must be a closed solid (the demo arch is). The case is not modified: the line of every tooth (case coordinates), the size of the aligner, whether it is closed and manifold and the holes through its wall are returned and, with dir, the aligner is written there as binary STL (upper_trimmed.stl, lower_trimmed.stl). Parameters: arch (all, upper or lower), dir, mode (equator, straight, scalloped or smooth), insertion_axis, equator_offset_mm, extension_mm, margin_gap_mm, scallop_mm (default the trim settings of the case), round_mm, thickness_mm, clearance_mm (default the case settings), voxel_mm, blockout_mm.

## ---- commands: stamp ----
aligners-cmd-stamp-doc = Puts the identification text of an aligner (arch, aligner number, date and case code, for example SUP 03 / 131026 / CODE) on the scan of an arch: embossed (kind model, the default) or engraved (kind shell). The text is the outline of a bundled font (B612 Mono Bold) turned into a distance field and applied along the surface normals to the triangles under it, which are refined to resolution_mm: the mesh keeps its topology (closed stays closed and manifold) and its labels. It goes on a molar or second premolar of the segmented arch, found like the plugin does (the tooth chosen first, the lingual face first, tilted towards the occlusal side until it fits, the case code dropped if the text is too big), or at a point given by hand in case coordinates (placement mode manual, which needs no segmentation). The case is not modified: the result is described (tooth, face, size of the text) and, with dir, written there as binary STL (upper_stamped.stl, lower_stamped.stl). Parameters: arch (all, upper or lower), dir, text (a template: the fields arch, number, passive, date and code go in curly braces), number, passive, code, date (YYYY-MM-DD, default today), date_format (DDMMYY, DD.MM.YY or YYMMDD), height (mm), depth (mm), min_wall, margin, clearance, kind (model or shell), placement (an object with mode tooth, plus tooth M6R, face lingual and read inside, or with mode manual, plus point, normal and up), method (local or sdf), resolution_mm.

## ---- commands: export ----
aligners-cmd-export-doc = Writes the aligners into a directory: the scan of each arch with the teeth at every stage as binary STL (upper_stage_00.stl is the initial situation; the gingiva follows the teeth when the arch has its gingiva, else it stays where it is), stages.csv with the movement of every tooth at every stage and the dates of the aligner, and visits.ics with the aligner changes of both arches as an iCalendar file. The case is not modified. Needs the staging. Parameters: dir (required), start_date (YYYY-MM-DD, default today), interval_days, policy (fixed or adaptive), lang (en or it), title.
aligners-cmd-report-doc = Full export package into a directory, in parallel: the model of each stage and the aligner shells (SUP/INF files in modelli_termoformatura and stampa_resina, passive aligners included), the print check of every mesh (closed, manifold, no self-intersections, minimum wall, fits the build plate), stages.csv, visits.ics, the patient programme and the print manifest of each arch, report.html (diagnosis, set-up, IPR, movements, staging, calendar, files, print check, references; with an occlusal and a frontal render), print_check.json and manifest.json with the BLAKE3 hash of every file. The case is not modified. Needs the staging. Parameters: dir (required), start_date (YYYY-MM-DD, default today), interval_days, policy, lang (en or it: language of the report, the programmes and the calendar), title, workflow (both, thermo or direct), print_check, self_intersections, min_thickness_mm, build_plate_mm, report, embed_renders, threads, generated (YYYY-MM-DD, fixes the date written in the files).
aligners-cmd-print-check-doc = Checks that the models of some stages are ready to print: closed and manifold surface, consistent orientation, one piece, no self-intersections, no hole through the wall, positive volume, fits the build plate. Writes nothing; the case is not modified. Needs the staging. Parameters: stages (default the first and the last), self_intersections, build_plate_mm.
aligners-cmd-manifest-doc = Verifies an export folder against its manifest.json (BLAKE3 hash and size of every file, missing and unlisted files), or with verify=false writes a manifest of the files found. Parameters: dir (required), verify.

## ---- commands: auto ----
aligners-cmd-auto = Whole pipeline
aligners-cmd-auto-doc = Runs the whole automatic pipeline on the scans of the case, with the default parameters, in order: orientation, automatic seeds of the arches that have none, segmentation, closed crowns (with the tooth frames), gingiva, diagnosis, setup, staging and change calendar; with both arches the virtual bite and the contact map; with export_dir the export. The first step that fails stops the pipeline: the error names the step and the case is left as it was. Returns every step with its seconds and its output. Parameters: seeds (missing: only the arches without seeds, all: replace them, keep: never), third_molars, missing (FDI numbers of missing teeth), occlusion (default true), start_date (YYYY-MM-DD, default today), interval_days, export_dir, full_report (the full export package instead of the models, the table and the calendar).

## ---- commands: visits ----
aligners-cmd-visits-doc = Analyses a control visit. The progress scan (scan: path of an STL, OBJ or PLY file) is placed on the situation the plan expects after the aligner stage the patient wears (default: the last aligner of the arch; 0 is the initial situation), every tooth of the plan is registered to the scan, and the position it really has is compared with the planned one. A tooth is on track when every component of the difference is within 0.5 mm and 2 degrees; the efficacy is the achieved over the planned movement. If every tooth is on track the plan goes on, otherwise a refinement is suggested and the teeth that are behind get a lower velocity (lag_speed) in the new aligners. The scan, placed on the plan and segmented with seeds on the achieved crowns (segment, on by default), and the figures of every tooth are stored as a visit of the case. The plan is not modified. Needs the teeth stage and, for a stage above 0, the staging. Parameters: scan (required), arch (needed when the case has both), date (YYYY-MM-DD, default today), stage, linear_mm, angular_deg, lag_speed, segment.
aligners-cmd-tracking-doc = Re-plans from where the treatment really is. The aligners already delivered are kept and a new series starts after them, towards the same final setup, numbered from the next aligner on. The starting position of every tooth is the one measured at a control visit (visit: its number, the last visit of the arch by default; the teeth that were behind move at the visit's lower velocity factor) or, without a scan, the planned position at the delivered aligner stage. The IPR already done is not planned again. Replaces the aligners after the delivered ones for the arch and keeps the other arch. Needs the setup and the plan or a visit. Parameters: arch, visit, stage (not together with visit), pattern.

## ---- commands: manual ----
aligners-cmd-manual-brush-doc = Paints or erases tooth labels on the scan by hand, at full resolution. The target is the union of vertices (vertex indices of the scan), the dab at centre (case coordinates, radius_mm) and the stroke that goes from centre through the points of stroke (a dab every half radius); view_dir, the direction the viewer looks along, leaves out the surface that looks away from the viewer. mode paint (default) gives the target the tooth number label (0 = gingiva); erase sends it back to the gingiva (with a label, only the vertices that have it); fill gives the label to the whole crown around vertex (or the vertex nearest to centre) up to its margin, found by a random walker on the folds of the scan (label 0 takes away the part of the clicked tooth beyond its crown). only_over restricts the change to the vertices that have that label now. The labels replace those of the arch (all gingiva when it was not segmented); a tooth whose region changes after its solid was built is listed in to_rebuild. Parameters: arch, label, mode, vertices, centre, radius_mm, stroke, view_dir, vertex, only_over.
aligners-cmd-manual-clean-doc = Cleans the tooth labels after the brush: the detached pieces of a tooth smaller than a quarter of its main piece and the small holes of gingiva inside a crown take the label of their surroundings, and the jagged border is smoothed. Parameters: arch (default every segmented arch), fdi (one tooth, default all), rings (smoothing rings, 0 to 3, default 1).
aligners-cmd-manual-renumber-doc = Changes the FDI number of a tooth; when the new number is already used, the two teeth swap their numbers. The seeds exchange their numbers, the labels of the arch are renamed, the renumbered teeth are measured and closed again (their number decides their axes and centre of resistance) and what the setup and the staging had for them is dropped. Both numbers must be in the same arch. Parameters: src (required), dst (required).
aligners-cmd-manual-missing-doc = Marks a tooth as missing (agenesis, extraction): its seed, labels, frame, solid and movements are removed, its space stays empty, and the number is kept in the list of missing teeth, which the seeds and the brush respect. With missing false the tooth is present again (to be seeded or painted, then rebuilt). Parameters: fdi (required), missing (default true).
aligners-cmd-manual-seeds-doc = Adds, moves or deletes the seed of a tooth. A seed is the centre of the occlusal face of a tooth with its number; the segmentation starts from the seeds, so the labels of the arch are out of date after an edit and the segmentation must be run again. Parameters: op (add, move or delete), fdi (required), position (case coordinates in mm, required to add or move), snap_to_scan (put the seed on the nearest vertex of the scan).
aligners-cmd-manual-rebuild-tooth = Rebuild one tooth by hand
aligners-cmd-manual-rebuild-tooth-doc = Builds a single tooth again from its region on the scan: from the current labels, from a selection of vertices that becomes the tooth (its other vertices go to the gingiva), or after finding the region again from the seed of the tooth (resegment). The region is checked first (size, height, border of the scan, holes) and an invalid one is refused with the reason, changing nothing. The teeth of the arch are then measured again and the closed solid of the tooth replaces the old one; the other teeth keep their solids, frames and movements, and the setup and staging of this tooth are out of date. Parameters: fdi (required), vertices, resegment, neck_depth_mm, apex_mm, neck_scale.
aligners-cmd-manual-adjust-doc = Moves and turns one tooth by hand, in the final position (setup) or at one aligner (stage). The movement is an increment in the frame of the tooth, about its centre of resistance: mesial_mm, buccal_mm, extrusion_mm, torque_deg, tip_deg, rotation_deg, and/or a pose delta (t in mm, q as a quaternion x, y, z, w; absolute makes it the new pose; reset brings the tooth back to rest first). The result is stored as the other stages store it. The crown is then checked against its neighbours with the collision metric of the staging: the penetration with each neighbour is reported before and after, and refuse_on_collision turns a collision into an error. A move of the setup leaves the staging out of date; lock keeps the tooth where it is put when the setup is computed again. Parameters: fdi (required), stage, the six components, delta, absolute, reset, lock, refuse_on_collision.

## ---- commands: study_base ----
aligners-menu-output-study-models = Study models (European / ABO)
aligners-desc-output-study-models = Orthodontic study models on a trimmed base, in the European or the American (ABO) style chosen in the settings.
aligners-cmd-study-models-doc = Builds the orthodontic study models of the arches that have a scan and tooth labels, in occlusion (after the bite registration or the virtual bite), on a trimmed base. Common to both styles: base planes parallel to the occlusal plane, a back plane perpendicular to the midpalatal raphe (estimated from the symmetry of the upper arch) shared by both arches, vertical walls, and the anatomy (teeth, gingiva down to the sulcus, palate) never cut: the base is lowered and the walls pushed out instead, with a warning. ABO: 70 mm for the articulated pair, one third art portion, upper heptagon with heels at 115 degrees, sides at 65 degrees and the front cuts meeting on the raphe, lower front arc from canine to canine. European: walls at 90 degrees, 10 mm base under the anatomy, upper tip on the raphe, lower front flat. The parameters start from the case settings; every field given overrides them. With dir it writes study-model-upper.stl and study-model-lower.stl there. Returns the frame, the planes, the outline, the angles and heights designed and measured on the meshes, and the warnings. The case is not modified. Parameters: arch, dir, style, total_height_mm, art_fraction, art_thickness_mm, base_thickness_mm, m_base_mm, back_margin_mm, side_angle_deg, lower_side_angle_deg, heel_angle_deg, heel_length_mm, front_angle_deg, side_clear_mm, canine_clear_mm, labial_clear_mm, vestibule_clear_mm, eu_side_angle_deg, eu_heels, sulcus_reach_mm, voxel_mm, tolerance_mm.
aligners-cmd-study-models-settings = Study model settings
aligners-cmd-study-models-settings-doc = Changes the study model settings of the case: the fields given replace the current ones and become the defaults of the study models. Parameters: style (european or abo), total_height_mm, art_fraction, base_thickness_mm, m_base_mm, back_margin_mm, side_angle_deg, heel_angle_deg, side_clear_mm, vestibule_clear_mm.
aligners-study-settings = Base settings
aligners-study-style = Style
aligners-study-style-european = European
aligners-study-style-abo = American (ABO)
aligners-study-total-height = Pair height, ABO (mm)
aligners-study-art-fraction = Art portion, ABO (fraction)
aligners-study-base-thickness = Base thickness, European (mm)
aligners-study-m-base = Clearance under the anatomy (mm)
aligners-study-back-margin = Back margin (mm)
aligners-study-side-angle = Side cut angle, ABO (deg)
aligners-study-heel-angle = Heel angle (deg)
aligners-study-side-clear = Side clearance, ABO (mm)
aligners-study-vestibule-clear = Vestibule clearance, European (mm)
aligners-study-apply = Apply
aligners-study-reset = Revert
aligners-study-no-case = Open a case with the Aligners data to edit the settings.

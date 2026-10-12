# thaum-painter design truths

### 2026-09-11T17-27-44Z-game-art-workflow
- j-quote: "actually the painter looks pretty well built, this is less than i thought there would be. lets focus on getting the game and the painter potentially talking to eachother graphically so i can make assets in painter and have those assets in the game. ill end up doing many things. storing animations, general shapes and paintings, generating land, creating examples for models to look aat and trhen use based on my artfiles. materials and sprites are a larger party here than in the painter right now, but both of those are in."
- interpretation: Painter-to-game graphical asset interchange is the next focus; animated, general-art, terrain, model-reference, material, and sprite workflows need cleanly separated ownership.

### 2026-09-11T18-30-59Z-external-asset-bridge
- j-quote: "this asset bridge should probably not be the an integral part of the painter because the painter's lied to other people and they're going to roduce files so this should really just be like looking at files finding out how to turn them into assets."
- interpretation: Painter files remain independently produced source documents; a separate consumer owns interpretation and compilation into downstream assets.

### 2026-09-11T18-34-10Z-game-sprite-material-persistence
- j-quote: "Sprite and material values are made for the game specifically. Those will have to survive and those have to be available in the painter as well we haven't really extended out the sprites yet.."
- interpretation: Painter documents must persist game-targeted sprite and material references, while the painter presents the same externally owned values for authoring.

### 2026-09-11T20-49-23Z-native-game-cell-language
- j-quote: "yes ideally in the same way the painter does. The painter needs to support these natively and equally to glyphs and flat color"
- j-quote: "painter will have sprites and glyphs, and materials and flat color. Those exist in the game as well. They should share a language."
- interpretation: Painter persistence and authoring must treat game sprite and material cell values as first-class peers of glyph and flat color, using the same cell-value language as the game.

### 2026-09-11T20-49-23Z-flat-export-and-fixed-origin
- j-quote: "we will have different export settings eventually so layers can be interpolated different. Lets do one export called \"flat\" and it can export the whole 3d cell with animation but no layers intact. Just what the renderer would render. Needs to include interpolations!!!"
- j-quote: "layers should all have like a an origin location like a point where the asset is like fixated from we're going to use that to rotate later. It's probably already in because we have moved per layer that's essentially what I'm talking about this like center point of the content. We're going to need a center point that doesn't move though it needs to be fixed in a way where I can have something animate around and even if it's one cell it'll keep its center point if it's animated off of that."
- interpretation: The first export profile is `flat`: a renderer-resolved, animated, interpolation-preserving 3D-cell result with no retained layers. Each authored layer also needs an invariant origin/pivot, separate from its animated movement.

### 2026-09-11T20-49-23Z-semantic-layers-not-collision-export
- j-quote: "for things like collision I could see this being like a special expert where I could like collision is set up with tags in the game so in cases like this we wouldn't really be exporting collision we might be exporting layers that use their names to dictate what those layers mean"
- interpretation: Collision remains game-tag semantics; painter export can preserve named layers for downstream semantic interpretation rather than baking collision data.

### 2026-09-11T21-21-17Z-transferable-assets-first
- j-quote: "whats the first step do you think towards getting the game extracted? it definitly starts in the assets and getting those transferable"
- interpretation: The first extraction milestone is transferable graphical assets between painter and game.

### 2026-09-11T21-56-56Z-painted-ordered-cell-shaders
- j-quote: "while we are changing the file systems, shaders. can they also be stored yet? ideally i can paint shaders on. shaders will be addiitive and have ordering. it will be per cell as well"
- interpretation: Painter files must persist an ordered additive shader stack per cell, and painter authoring must support applying shaders directly to cells.

### 2026-09-11T22-51-01Z-paintable-game-shaders-only
- j-quote: "yes lets exclude those. we should allow for specific shaders to be painted on. for instance, we will havce a shader in the game for things that are on fire in different increments. the different levels of fire shader will be able to painted on once they are made."
- interpretation: Painter exposes only asset-authorable shaders selected by the game; editor-only shaders stay unavailable. Distinct fire-intensity shaders become separately paintable once defined.

### 2026-09-12T07-48-05Z-clear-character-tools
- j-quote: "Lasso erase is just lasso with the clear cell / erase cell selected. Lets shift this around. We should allow the user to select clear or erase in any tool. It should be under character specifically. Then he user can brush, lasso, fill, ect. All with clear. Then it extents to new tools later too. Eraser is this but manual for brush."
- interpretation: Clear is a visible character-picker choice that equips the blank glyph; ordinary paint tools remove cells through their shared write seam, and the standalone Erase tool is retired.

### 2026-09-12T11-43-15Z-general-exporter-presets-and-filenames
- j-quote: "I think it should save the preset that the asset exporter uses. We talked about asset exporter as a third thing. Not renderer or painter."
- j-quote: "I'm thinking we need an exporter that is general. Not a thaumworld export, but more like a single-layer export so we know what kind of file it is. They won't have IDs quite yet, just filenames. IDs will happen in the game layer where we store these exports."
- interpretation: Painter documents retain generic exporter preset selection. A separate general exporter owns export execution and uses filenames, while each downstream game assigns IDs when it stores exports.

### 2026-09-12T11-47-57Z-document-flat-export-section
- j-quote: "No look back in this chat. You are missing something every layer does not need a single export, It was for a flat export section."
- interpretation: The initial document-level export configuration is the `flat` export section: it emits the complete renderer-resolved document, not an automatic one-export-per-layer policy.

### 2026-09-12T12-11-49Z-flat-preserves-interpolation
- j-quote: "flat preserves interpolation."
- interpretation: The `flat` export representation retains document interpolation semantics rather than baking only discrete sampled frames.

### 2026-09-12T13-25-00Z-facing-as-quantized-rotation
- j-quote: "lets look at, and think about facing. we used to have this in the game. its an important concept. later on i could see doing things like custom cells that shade differently based on viewing angle. i think rotations could be smooth if we did some raster intertpolation (we already have that) instead of aliasing. then we could do slight rotations and such. facing on layers would allow us to do some crazy things, like track rotations. that would lead into the rotation property row nicely."
- interpretation: Layer facing should be one system with rotation: a `rotation` property track on each layer (hold/interpolate/ease like move) whose discrete 90-degree steps resolve to the renderer's six CellGroupFacing cardinals, and whose in-between frames blend cell content between the two bracketing facings through the raster interpolation seam (interp_raster/shape-fade) instead of snapping — smooth slight rotations without aliasing. Viewer-relative shading (per-view-angle cell shading, the old game's view_direction art variants) stays separate: it belongs in the ordered per-cell shader stack, with view angle later passed into the renderer's resolve_shaded_* seam. Build order: facing field on layer + schema, facing row in the layers panel, rotation track, view-angle shader input.

### 2026-09-12T13-35-00Z-united-facing-seam
- j-quote: "facing im not convinced its set up properly in the old system. it was messy. some cells may retain facing information with special cells that shade differently on different facing setups. like an asymmetrical object. that is what this is for. it makes sense for these to have set cells per facing directions. like chests in the old game. layers probably need a facing in general as well so that they can get rotated."
- interpretation: One facing vocabulary (the renderer's six CellGroupFacing cardinals) carried two ways and united at one resolution seam. Carrier 1, per layer: a facing field on the painter layer + file schema feeding CellGroup::with_facing — the quantized rotation carrier. Carrier 2, per cell: facing-variant ASSETS, not per-cell facing fields — a cell references one asset (like a chest) whose definition holds a set of cells per facing direction, so asymmetrical objects stay one reference. The union point is an effective-facing resolution seam in the renderer domain: layer facing combined with the viewer's view orientation yields one value that both facing-variant asset resolution and (later) view-angle shader shading consume. Old-game north/south/east/west/up/down maps onto the cardinals through one owned mapping. Painter files stay source-facing: they store the facing field and the asset reference, never the resolved variant.

### 2026-09-12T13-50-00Z-facing-piece-alignment
- j-quote: "there 100% needs to be both facing of camera, and facing of objects in the game. its quite important. the design thing im trying to figure out is sometimes cells themselves might need facing bc asymmetrical objects. i know its not standard but id like it done anyway... i need per cell facing but who should opt in? if i opt in does it get messy? i also have per camera angle facing. this is necessary and will be sick."
- interpretation: Five aligned facing pieces. (1) Camera facing: renderer camera, six directions, runtime-only. (2) Object facing: layer field -> CellGroup::with_facing, the rotation carrier. (3) Per-cell facing: NO per-cell field and NO tags-style opt-in — cells stay plain Sprite{asset_file} references; the referenced ASSET optionally declares a per-facing cell set with same_as fallbacks, so opt-in happens once per asset file and facing-invariant cells cost nothing. (4) One resolution seam in renderer domain takes (group facing, camera facing, asset keying: object|viewer) and returns the cell set — the only place the two facing sources combine; object keying = chest-style art that turns with the object, viewer keying = per-camera-angle art like two-sided signs. (5) View-angle shading: shader stack receives the same facing context later. Old-game north/south/east/west/up/down maps to the six cardinals through one owned function. Game tags stay game-semantics (collision/structure) and never carry facing.

### 2026-09-12T14-05-00Z-universal-cell-facing
- j-quote: "if it was single source of truth like all cells had a facing, would we blow up the system file sizes much? maybe we could store it as something small. ideally everything that uses facing uses all facing bc all levels of facing matter. camera effects a cells relative facing additively so things actually look 3d in each relative view"
- interpretation: DECIDED over the asset-keying opt-in design: facing is a universal per-cell field (default PosZ, serde skip-if-default so default cells cost zero bytes), joining weight/texture/warble as an always-present cell axis — the ASCII medium's normal vector. Relative facing composes additively through the cube rotation group (24 elements, integer-only): camera-inverse composed with layer facing composed with cell facing. This dissolves the object-vs-viewer keying declaration entirely — chest-latch and two-sided-sign behavior are the same lookup rule under different compositions. Variant sprite assets remain the carrier of per-facing cell sets (same_as fallbacks), keyed by composed relative facing. Build order: compose algebra in renderer domain first (exhaustively tested), then cell/layer/schema fields, then variant assets, rotation track, painter UI, view-angle shading.

### 2026-10-10T19-40-14Z-file-truth-vs-user-truth-color-split
- j-quote: "I think calling them dynamic versus static is actually probably not a good idea. Because we don't really need to have dynamic colors within drawings like we could just save the RGB of what the color is and that would give us more in-depth fill information later on like tolerance. That would be nice to have I don't think we need to call it dynamic for systematic I think file Truth versus user truth is just fine."
- interpretation: The color split is named file truth (saved with the file) vs user truth (per-user UI, stable RGB). Cells always store raw RGB, never palette indices, so depth like tolerance fill stays open.

### 2026-10-10T19-40-14Z-palette-module-single-interface
- j-quote: "then they also need a new feature on the color palette module where a user can hit like there needs to be a section where they can input probably either an RGB value or a hex value and then hit plus and it would add that color to the palette. Then the user can select the color from the palette and work from there. The color block should automatically add and subtract colors when it the user is changing the those colors through the color palette module the color palette module should really be the way users are in like we should build it as one interface to change those index colors just what's simple for now. It will change the file oriented colors."
- interpretation: The palette module is the single interface for editing the file's index colors: hex/RGB input + plus adds to the file palette, minus removes without rewriting cells, and the color block's banding auto add/subtracts as the palette changes.

### 2026-10-10T19-40-14Z-default-palette-legacy-37
- j-quote: "A default palette can be used for the current one we have. User is also going to need to be able to switch out pallets later on so if we keep them as like lists of colors on that palette I think that would be clean and we don't have to build the switching of pallets later but we could have a default palette which is the hard coded list that we had before and then that could be everyone's like starting point when they download the program which I think is fine. We could add later additional presets"
- interpretation: Palettes are plain ordered color lists; the shipped default is the hardcoded legacy 37 as every user's starting point. Palette switching and extra presets are later work, not built now.

### 2026-10-11T00-00-00Z-palette-typed-entry-linked-hex-rgb
- j-quote: "i cant rely on the user having the color already, we need to allow the user to type in either an rgb value or a hex code for a color. ... maybe we have an entry space for hex, and an entry space for rgb values. changing one >< changes the other. when i press + it adds the colros on either of those entries (bc we keep them linked) into the indexed color palette."
- interpretation: The palette module's entry takes a hex value or an rgb value; the two fields are two views of one linked color, and `+` commits that color into the indexed palette. The entry no longer depends on the hand already holding the color.

### 2026-10-11T00-00-00Z-palette-buttons-inside-module
- j-quote: "also the ui is weirtd for the color pallette. the buttons for adding and subtracting colors are below the color pallette module. ... also the buttons have to move inside this moduele."
- interpretation: The `−`/`+` buttons and the entry fields live inside the COLORS panel's own bounds as its lower strip rows — nothing floats below the module.

### 2026-10-11T00-00-00Z-palette-no-click-reordering
- j-quote: "also the reorderiong of colros in the pallette module actualyl makes it really annoying to use. lets have that reordering not hjappen. just a list. mayhbe derrive the order from the color itself like value + hue sort to keep it simple and reactive with module size."
- interpretation: Swatch clicks only select; the list order is derived from the colors themselves (value descending, hue ascending) and reflows with the module's size — no most-recently-used reordering.

### 2026-10-11T00-00-00Z-color-block-clamp-widget-free-pick-flow
- j-quote: "The widget would toggle on and off the index color clamping on the color block. When the color block is not clamped the user could press a color that's outside of the pallet in order to select it. This would be incredibly helpful because then from there they could press plus to add a custom color and this forgive users a lot of control. This would even allow users to paint with colors not on a pallet they could paint a little more like naturally. And it also means that we could also run it through the color block module which is a module that we put some ux Polish into and will continue to further polish."
- interpretation: The color block's clamp widget turns off index clamping so any pick carries raw to the hand — painting off-palette works naturally, and `+` in the palette module adds the custom color. Free picking lives on the color block, the module getting ongoing UX polish.

### 2026-10-11T00-00-00Z-fill-3way-connectivity
- j-quote: "So fill goes from adjacent and diagonal toggle to a 3 way toggle of adjacent, diagonal, and all. It also has an additional toggle that allows for selecting with depth. Depth should be responsive to adjacent diagonal and all (non continuous) for when 3d is toggled on."
- interpretation: Fill connectivity is a 3-way choice (adjacent/diagonal/all), and `all` means non-continuous — every matching cell in the region. The depth toggle gates the fill's reach across 3d layers and works under every connectivity.

### 2026-10-11T00-00-00Z-fill-lasso-shared-3d-toggle
- j-quote: "We need a toggle for effecting 3d layers. That would allow users to use the flood fill tool to fill multiple 3D layers relative to their viewing depth and angle. Lasso will also get a 3d toggle it will simply do the lasso tool action but for every depth layer relative to the viewers viewing angle. They can share the same property for 3d toggle. Ill have this on brush later as well."
- interpretation: Fill and lasso share one per-hand 3d property (`fill_3d`); depth is always relative to the viewer's viewing angle, and brush is the later adopter of the same property.

### 2026-10-11T00-00-00Z-picker-rows-wrap-short-of-the-scrollbar
- j-quote: "on the character pallette module the scrollbar goes overthe last character on the rol. the character ordering needs to be aware of the scroll bar so it can resize depending on if its there or not and not make characters impossible to access without resizing the module."
- interpretation: when the vertical scrollbar is visible the graphic picker's character rows wrap one column short so the bar's column stays bar-only — every character stays reachable without resizing the module.

### 2026-10-11T00-00-00Z-toolbox-scrolls-its-tool-rows
- j-quote: "controlls needs to get a better scrollbar,. its broken there in the saemw ayt that the perspectivcve module was. make sure tools alsop has a scrollbar if it needs one / is too short to display all toolsl. we will be adding another tool shortly and its going to expand as well."
- interpretation: the controls panel gets the same span-math fix the perspective module got, and the toolbox scrolls its tool rows through the shared scrollbar seam whenever the panel is too short — built for the tool list to keep growing.

### 2026-10-11T00-00-00Z-user-session-state-resets-on-unreadable
- j-quote: "It would be better if user state just reset to the default if it couldn't be saved. Things like selected tools are not really valuable saves"
- interpretation: An unreadable or unparsable user session state file is discarded and boots as the default (best-effort removal so the next save rewrites it) — session-state parse failures must never block boot or require legacy field compat like fill_diagonal.

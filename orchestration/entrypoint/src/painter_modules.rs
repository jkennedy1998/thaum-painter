//! Painter module wiring: the one place that assembles the app's module
//! registry. The event loop consumes shared handles (tool state, selection,
//! layers panel, number-field edit) and never touches registration order.

use std::cell::RefCell;
use std::rc::Rc;

use thaum_painter_domain::{
    GraphicPickerModule, HandSettingsModule, LayersPanelModule, LayersPanelState,
    MaterialPickerModule, PaintCanvasBoundsModule, PaintColorBlockModule, PaintColorPickerModule,
    PaintTool, PainterSelection, ToolDef, ToolState, ToolboxModule,
};
use thaum_renderer_domain::{
    conflicting_actions, effective_bindings, format_raw_input, ActionBindingMap, ControlsProfile,
    ControlsPanelModule, ModuleRect, ModuleRegistry, NumberFieldEdit, UiCustomizationModule,
    UiPalette,
};

/// Registers every painter module and returns the registry. Shared handles
/// are created by the caller so the event loop and the modules both hold
/// them; this function only wires them into their panels.
#[allow(clippy::too_many_arguments)] // wiring fn: every handle is a distinct live session object
pub(crate) fn build_painter_modules(
    tool_state: &Rc<RefCell<ToolState>>,
    selection: &Rc<RefCell<PainterSelection>>,
    number_edit: &Rc<RefCell<Option<NumberFieldEdit>>>,
    layers_panel_state: &Rc<RefCell<LayersPanelState>>,
    paint_canvas_viewport: &Rc<RefCell<ModuleRect>>,
    drawing_space_wheel_mode: &Rc<RefCell<
        thaum_painter_domain::DrawingSpaceWheelMode,
    >>,
    controls_profile: &Rc<RefCell<ControlsProfile>>,
    effective_painter_bindings: &Rc<RefCell<ActionBindingMap>>,
    painter_bindings: &ActionBindingMap,
    ui_palette: &UiPalette,
) -> ModuleRegistry {
    let mut modules = ModuleRegistry::new();
    modules.register(Box::new(
        ToolboxModule::new(
            "painter_toolbox",
            ModuleRect {
                x0: -6,
                y0: -1,
                x1: 12,
                y1: 5,
            },
            tool_state.clone(),
            thaum_painter_domain::painter_tools::all()
                .iter()
                .filter_map(|descriptor| {
                    Some(ToolDef {
                        tool: PaintTool::from_id(descriptor.id)?,
                        icon: descriptor.icon,
                        label: descriptor.label,
                    })
                })
                .collect(),
        )
        .with_palette(ui_palette.clone()),
    ));
    modules.register(Box::new(PaintColorPickerModule::new(
        "painter_color_picker",
        ModuleRect {
            x0: 13,
            y0: -8,
            x1: 29,
            y1: 0,
        },
        tool_state.clone(),
        ui_palette.clone(),
    )));
    modules.register(Box::new(PaintColorBlockModule::new(
        "painter_color_block",
        ModuleRect {
            x0: 13,
            y0: -21,
            x1: 31,
            y1: -10,
        },
        tool_state.clone(),
        ui_palette.clone(),
    )));
    modules.register(Box::new(
        MaterialPickerModule::new(
            "painter_material_picker",
            ModuleRect {
                x0: 32,
                y0: -21,
                x1: 48,
                y1: -10,
            },
            tool_state.clone(),
        )
        .with_palette(ui_palette.clone()),
    ));
    modules.register(Box::new(
        GraphicPickerModule::new(
            "painter_graphic_picker",
            ModuleRect {
                x0: 31,
                y0: -8,
                x1: 70,
                y1: 28,
            },
            tool_state.clone(),
        )
        .with_palette(ui_palette.clone()),
    ));
    modules.register(Box::new(
        HandSettingsModule::new(
            "painter_hand_settings",
            ModuleRect {
                x0: 13,
                y0: 2,
                x1: 43,
                // 14 tall: content 11 = 10 visible property rows + the
                // reserved bottom hand-color row.
                y1: 16,
            },
            tool_state.clone(),
            selection.clone(),
            number_edit.clone(),
        )
        .with_palette(ui_palette.clone()),
    ));
    modules.register(Box::new(PaintCanvasBoundsModule::new(
        "paint_canvas_bounds",
        paint_canvas_viewport.clone(),
        drawing_space_wheel_mode.clone(),
        ui_palette.clone(),
    )));
    modules.register(Box::new(
        LayersPanelModule::new(
            "painter_layers_panel",
            ModuleRect {
                x0: 25,
                y0: -24,
                x1: 70,
                y1: -3,
            },
            layers_panel_state.clone(),
        )
        .with_palette(ui_palette.clone()),
    ));
    modules.register(Box::new(UiCustomizationModule::new(
        "painter_ui_customization",
        ModuleRect {
            x0: 44,
            y0: 2,
            x1: 66,
            y1: 12,
        },
        ui_palette.clone(),
        {
            let tool_state = tool_state.clone();
            move || {
                let rgb = tool_state.borrow().left_hand.color.preview_rgb();
                [rgb.0, rgb.1, rgb.2]
            }
        },
        {
            let tool_state = tool_state.clone();
            move || {
                let rgb = tool_state.borrow().right_hand.color.preview_rgb();
                [rgb.0, rgb.1, rgb.2]
            }
        },
    )));
    modules.register(Box::new(ControlsPanelModule::new(
        "painter_controls_panel",
        ModuleRect {
            x0: 4,
            y0: 13,
            x1: 44,
            y1: 47,
        },
        ui_palette.clone(),
        crate::painter_control_rows(),
        {
            let effective = effective_painter_bindings.clone();
            move |action| {
                effective
                    .borrow()
                    .bindings_for(action)
                    .first()
                    .map(format_raw_input)
                    .unwrap_or_else(|| "unbound".to_string())
            }
        },
        {
            let effective = effective_painter_bindings.clone();
            move |action| {
                conflicting_actions(&effective.borrow(), action)
                    .iter()
                    .filter_map(|other| {
                        effective
                            .borrow()
                            .bindings_for(other)
                            .first()
                            .map(format_raw_input)
                    })
                    .collect()
            }
        },
        {
            let profile = controls_profile.clone();
            let effective = effective_painter_bindings.clone();
            let defaults = painter_bindings.clone();
            move |action, binding| {
                profile.borrow_mut().set_override(action, binding);
                *effective.borrow_mut() = effective_bindings(&defaults, &profile.borrow());
            }
        },
    )));
    modules
}

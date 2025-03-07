use crate::consts::{DRAG_THRESHOLD, LINE_ROTATE_SNAP_ANGLE};
use crate::document::DocumentMessageHandler;
use crate::frontend::utility_types::MouseCursorIcon;
use crate::input::keyboard::{Key, MouseMotion};
use crate::input::mouse::ViewportPosition;
use crate::input::InputPreprocessorMessageHandler;
use crate::layout::widgets::{LayoutRow, NumberInput, PropertyHolder, Widget, WidgetCallback, WidgetHolder, WidgetLayout};
use crate::message_prelude::*;
use crate::misc::{HintData, HintGroup, HintInfo, KeysGroup};
use crate::viewport_tools::snapping::SnapHandler;
use crate::viewport_tools::tool::{DocumentToolData, Fsm, ToolActionHandlerData};

use graphene::layers::style;
use graphene::Operation;

use glam::{DAffine2, DVec2};
use serde::{Deserialize, Serialize};

#[derive(Default)]
pub struct Line {
	fsm_state: LineToolFsmState,
	data: LineToolData,
	options: LineOptions,
}

pub struct LineOptions {
	line_weight: u32,
}

impl Default for LineOptions {
	fn default() -> Self {
		Self { line_weight: 5 }
	}
}

#[remain::sorted]
#[impl_message(Message, ToolMessage, Line)]
#[derive(PartialEq, Clone, Debug, Hash, Serialize, Deserialize)]
pub enum LineMessage {
	// Standard messages
	#[remain::unsorted]
	Abort,

	// Tool-specific messages
	DragStart,
	DragStop,
	Redraw {
		center: Key,
		lock_angle: Key,
		snap_angle: Key,
	},
	UpdateOptions(LineOptionsUpdate),
}

#[remain::sorted]
#[derive(PartialEq, Clone, Debug, Hash, Serialize, Deserialize)]
pub enum LineOptionsUpdate {
	LineWeight(u32),
}

impl PropertyHolder for Line {
	fn properties(&self) -> WidgetLayout {
		WidgetLayout::new(vec![LayoutRow::Row {
			name: "".into(),
			widgets: vec![WidgetHolder::new(Widget::NumberInput(NumberInput {
				unit: " px".into(),
				label: "Weight".into(),
				value: self.options.line_weight as f64,
				is_integer: true,
				min: Some(0.),
				on_update: WidgetCallback::new(|number_input| LineMessage::UpdateOptions(LineOptionsUpdate::LineWeight(number_input.value as u32)).into()),
				..NumberInput::default()
			}))],
		}])
	}
}

impl<'a> MessageHandler<ToolMessage, ToolActionHandlerData<'a>> for Line {
	fn process_action(&mut self, action: ToolMessage, data: ToolActionHandlerData<'a>, responses: &mut VecDeque<Message>) {
		if action == ToolMessage::UpdateHints {
			self.fsm_state.update_hints(responses);
			return;
		}

		if action == ToolMessage::UpdateCursor {
			self.fsm_state.update_cursor(responses);
			return;
		}

		if let ToolMessage::Line(LineMessage::UpdateOptions(action)) = action {
			match action {
				LineOptionsUpdate::LineWeight(line_weight) => self.options.line_weight = line_weight,
			}
			return;
		}

		let new_state = self.fsm_state.transition(action, data.0, data.1, &mut self.data, &self.options, data.2, responses);

		if self.fsm_state != new_state {
			self.fsm_state = new_state;
			self.fsm_state.update_hints(responses);
			self.fsm_state.update_cursor(responses);
		}
	}

	fn actions(&self) -> ActionList {
		use LineToolFsmState::*;

		match self.fsm_state {
			Ready => actions!(LineMessageDiscriminant; DragStart),
			Drawing => actions!(LineMessageDiscriminant; DragStop, Redraw, Abort),
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineToolFsmState {
	Ready,
	Drawing,
}

impl Default for LineToolFsmState {
	fn default() -> Self {
		LineToolFsmState::Ready
	}
}

#[derive(Clone, Debug, Default)]
struct LineToolData {
	drag_start: ViewportPosition,
	drag_current: ViewportPosition,
	angle: f64,
	weight: u32,
	path: Option<Vec<LayerId>>,
	snap_handler: SnapHandler,
}

impl Fsm for LineToolFsmState {
	type ToolData = LineToolData;
	type ToolOptions = LineOptions;

	fn transition(
		self,
		event: ToolMessage,
		document: &DocumentMessageHandler,
		tool_data: &DocumentToolData,
		data: &mut Self::ToolData,
		tool_options: &Self::ToolOptions,
		input: &InputPreprocessorMessageHandler,
		responses: &mut VecDeque<Message>,
	) -> Self {
		use LineMessage::*;
		use LineToolFsmState::*;

		if let ToolMessage::Line(event) = event {
			match (self, event) {
				(Ready, DragStart) => {
					data.snap_handler.start_snap(document, document.bounding_boxes(None, None), true, true);
					data.drag_start = data.snap_handler.snap_position(responses, input.viewport_bounds.size(), document, input.mouse.position);

					responses.push_back(DocumentMessage::StartTransaction.into());
					data.path = Some(document.get_path_for_new_layer());
					responses.push_back(DocumentMessage::DeselectAllLayers.into());

					data.weight = tool_options.line_weight;

					responses.push_back(
						Operation::AddLine {
							path: data.path.clone().unwrap(),
							insert_index: -1,
							transform: DAffine2::ZERO.to_cols_array(),
							style: style::PathStyle::new(Some(style::Stroke::new(tool_data.primary_color, data.weight as f32)), None),
						}
						.into(),
					);

					Drawing
				}
				(Drawing, Redraw { center, snap_angle, lock_angle }) => {
					data.drag_current = data.snap_handler.snap_position(responses, input.viewport_bounds.size(), document, input.mouse.position);

					let values: Vec<_> = [lock_angle, snap_angle, center].iter().map(|k| input.keyboard.get(*k as usize)).collect();
					responses.push_back(generate_transform(data, values[0], values[1], values[2]));

					Drawing
				}
				(Drawing, DragStop) => {
					data.drag_current = data.snap_handler.snap_position(responses, input.viewport_bounds.size(), document, input.mouse.position);
					data.snap_handler.cleanup(responses);

					match data.drag_start.distance(input.mouse.position) <= DRAG_THRESHOLD {
						true => responses.push_back(DocumentMessage::AbortTransaction.into()),
						false => responses.push_back(DocumentMessage::CommitTransaction.into()),
					}

					data.path = None;

					Ready
				}
				(Drawing, Abort) => {
					data.snap_handler.cleanup(responses);
					responses.push_back(DocumentMessage::AbortTransaction.into());
					data.path = None;
					Ready
				}
				_ => self,
			}
		} else {
			self
		}
	}

	fn update_hints(&self, responses: &mut VecDeque<Message>) {
		let hint_data = match self {
			LineToolFsmState::Ready => HintData(vec![HintGroup(vec![
				HintInfo {
					key_groups: vec![],
					mouse: Some(MouseMotion::LmbDrag),
					label: String::from("Draw Line"),
					plus: false,
				},
				HintInfo {
					key_groups: vec![KeysGroup(vec![Key::KeyShift])],
					mouse: None,
					label: String::from("Snap 15°"),
					plus: true,
				},
				HintInfo {
					key_groups: vec![KeysGroup(vec![Key::KeyAlt])],
					mouse: None,
					label: String::from("From Center"),
					plus: true,
				},
				HintInfo {
					key_groups: vec![KeysGroup(vec![Key::KeyControl])],
					mouse: None,
					label: String::from("Lock Angle"),
					plus: true,
				},
			])]),
			LineToolFsmState::Drawing => HintData(vec![HintGroup(vec![
				HintInfo {
					key_groups: vec![KeysGroup(vec![Key::KeyShift])],
					mouse: None,
					label: String::from("Snap 15°"),
					plus: false,
				},
				HintInfo {
					key_groups: vec![KeysGroup(vec![Key::KeyAlt])],
					mouse: None,
					label: String::from("From Center"),
					plus: false,
				},
				HintInfo {
					key_groups: vec![KeysGroup(vec![Key::KeyControl])],
					mouse: None,
					label: String::from("Lock Angle"),
					plus: false,
				},
			])]),
		};

		responses.push_back(FrontendMessage::UpdateInputHints { hint_data }.into());
	}

	fn update_cursor(&self, responses: &mut VecDeque<Message>) {
		responses.push_back(FrontendMessage::UpdateMouseCursor { cursor: MouseCursorIcon::Crosshair }.into());
	}
}

fn generate_transform(data: &mut LineToolData, lock: bool, snap: bool, center: bool) -> Message {
	let mut start = data.drag_start;
	let stop = data.drag_current;

	let dir = stop - start;

	let mut angle = -dir.angle_between(DVec2::X);

	if lock {
		angle = data.angle
	};

	if snap {
		let snap_resolution = LINE_ROTATE_SNAP_ANGLE.to_radians();
		angle = (angle / snap_resolution).round() * snap_resolution;
	}

	data.angle = angle;

	let mut scale = dir.length();

	if lock {
		let angle_vec = DVec2::new(angle.cos(), angle.sin());
		scale = dir.dot(angle_vec);
	}

	if center {
		start -= scale * DVec2::new(angle.cos(), angle.sin());
		scale *= 2.;
	}

	Operation::SetLayerTransformInViewport {
		path: data.path.clone().unwrap(),
		transform: glam::DAffine2::from_scale_angle_translation(DVec2::new(scale, 1.), angle, start).to_cols_array(),
	}
	.into()
}

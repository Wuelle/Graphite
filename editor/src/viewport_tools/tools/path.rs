use std::ops::Mul;

use crate::consts::SELECTION_THRESHOLD;
use crate::document::DocumentMessageHandler;
use crate::frontend::utility_types::MouseCursorIcon;
use crate::input::keyboard::{Key, MouseMotion};
use crate::input::InputPreprocessorMessageHandler;
use crate::layout::widgets::PropertyHolder;
use crate::message_prelude::*;
use crate::misc::{HintData, HintGroup, HintInfo, KeysGroup};
use crate::viewport_tools::shape_manipulation::ManipulationHandler;
use crate::viewport_tools::snapping::SnapHandler;
use crate::viewport_tools::tool::{DocumentToolData, Fsm, ToolActionHandlerData};

use glam::DVec2;
use graphene::intersection::Quad;

use serde::{Deserialize, Serialize};

#[derive(Default)]
pub struct Path {
	fsm_state: PathToolFsmState,
	data: PathToolData,
}

#[remain::sorted]
#[impl_message(Message, ToolMessage, Path)]
#[derive(PartialEq, Clone, Debug, Hash, Serialize, Deserialize)]
pub enum PathMessage {
	// Standard messages
	#[remain::unsorted]
	Abort,
	#[remain::unsorted]
	DocumentIsDirty,
	#[remain::unsorted]
	SelectionChanged,

	// Tool-specific messages
	DragStart {
		add_to_selection: Key,
	},
	DragStop,
	PointerMove {
		alt_mirror_toggle: Key,
	},
}

impl PropertyHolder for Path {}

impl<'a> MessageHandler<ToolMessage, ToolActionHandlerData<'a>> for Path {
	fn process_action(&mut self, action: ToolMessage, data: ToolActionHandlerData<'a>, responses: &mut VecDeque<Message>) {
		if action == ToolMessage::UpdateHints {
			self.fsm_state.update_hints(responses);
			return;
		}

		if action == ToolMessage::UpdateCursor {
			self.fsm_state.update_cursor(responses);
			return;
		}

		let new_state = self.fsm_state.transition(action, data.0, data.1, &mut self.data, &(), data.2, responses);

		if self.fsm_state != new_state {
			self.fsm_state = new_state;
			self.fsm_state.update_hints(responses);
			self.fsm_state.update_cursor(responses);
		}
	}

	// Different actions depending on state may be wanted:
	fn actions(&self) -> ActionList {
		use PathToolFsmState::*;

		match self.fsm_state {
			Ready => actions!(PathMessageDiscriminant; DragStart),
			Dragging => actions!(PathMessageDiscriminant; DragStop, PointerMove),
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathToolFsmState {
	Ready,
	Dragging,
}

impl Default for PathToolFsmState {
	fn default() -> Self {
		PathToolFsmState::Ready
	}
}

#[derive(Default)]
struct PathToolData {
	manipulation_handler: ManipulationHandler,
	snap_handler: SnapHandler,
}

impl Fsm for PathToolFsmState {
	type ToolData = PathToolData;
	type ToolOptions = ();

	fn transition(
		self,
		event: ToolMessage,
		document: &DocumentMessageHandler,
		_tool_data: &DocumentToolData,
		data: &mut Self::ToolData,
		_tool_options: &Self::ToolOptions,
		input: &InputPreprocessorMessageHandler,
		responses: &mut VecDeque<Message>,
	) -> Self {
		if let ToolMessage::Path(event) = event {
			use PathMessage::*;
			use PathToolFsmState::*;

			match (self, event) {
				// TODO: Capture a tool event instead of doing this?
				(_, SelectionChanged) => {
					// Remove any residual overlays that might exist on selection change
					data.manipulation_handler.remove_overlays(responses);

					// This currently creates new VectorManipulatorShapes for every shape, which is not ideal
					// Atleast it is only on selection change for now
					data.manipulation_handler.set_selected_shapes(document.selected_visible_layers_vector_shapes(responses));

					self
				}
				(_, DocumentIsDirty) => {
					// Update the VectorManipulatorShapes by reference so they match the kurbo data
					for shape in &mut data.manipulation_handler.selected_shapes {
						shape.update_shape(document, responses);
					}
					self
				}
				(_, DragStart { add_to_selection }) => {
					if data.manipulation_handler.has_selection {
						// Set the previous selected point to no longer be selected
						data.manipulation_handler.set_selection_state(false, responses);
					}
					// Select the first point within the threshold (in pixels)
					if data.manipulation_handler.select_manipulator(input.mouse.position, SELECTION_THRESHOLD, responses) {
						responses.push_back(DocumentMessage::StartTransaction.into());
						data.snap_handler.start_snap(document, document.visible_layers());
						let snap_points = data
							.manipulation_handler
							.selected_shapes
							.iter()
							.flat_map(|shape| shape.anchors.iter().map(|anchor| anchor.anchor_point_position()))
							.collect();
						data.snap_handler.add_snap_points(document, snap_points);
						Dragging
					}
					// We didn't find a point nearby, so consider selecting the nearest shape instead
					else {
						// Select shapes directly under our mouse
						let intersection = document
							.graphene_document
							.intersects_quad_root(Quad::from_box([input.mouse.position - DVec2::ONE, input.mouse.position + DVec2::ONE]));
						if !intersection.is_empty() {
							data.manipulation_handler.remove_overlays(responses);
							if input.keyboard.get(add_to_selection as usize) {
								responses.push_back(DocumentMessage::AddSelectedLayers { additional_layers: intersection }.into());
							} else {
								responses.push_back(
									DocumentMessage::SetSelectedLayers {
										replacement_selected_layers: intersection,
									}
									.into(),
								);
							}
						} else {
							// Clear the previous selection if we didn't find anything
							if !input.keyboard.get(add_to_selection as usize) {
								responses.push_back(DocumentMessage::DeselectAllLayers.into());
							}
						}
						Ready
					}
				}
				(Dragging, PointerMove { alt_mirror_toggle }) => {
					let should_not_mirror = input.keyboard.get(alt_mirror_toggle as usize);

					// Move the selected points by the mouse position
					let snapped_position = data.snap_handler.snap_position(responses, input.viewport_bounds.size(), document, input.mouse.position);
					let move_operation = data.manipulation_handler.move_selected_to(snapped_position, !should_not_mirror);
					responses.push_back(move_operation.into());
					Dragging
				}
				(_, DragStop) => {
					data.snap_handler.cleanup(responses);
					Ready
				}
				(_, Abort) => {
					data.manipulation_handler.remove_overlays(responses);
					Ready
				}
				(_, PointerMove { alt_mirror_toggle: _ }) => self,
			}
		} else {
			self
		}
	}

	fn update_hints(&self, responses: &mut VecDeque<Message>) {
		let hint_data = match self {
			PathToolFsmState::Ready => HintData(vec![
				HintGroup(vec![
					HintInfo {
						key_groups: vec![],
						mouse: Some(MouseMotion::Lmb),
						label: String::from("Select Point"),
						plus: false,
					},
					HintInfo {
						key_groups: vec![KeysGroup(vec![Key::KeyShift])],
						mouse: None,
						label: String::from("Add/Remove Point (coming soon)"),
						plus: true,
					},
				]),
				HintGroup(vec![HintInfo {
					key_groups: vec![],
					mouse: Some(MouseMotion::LmbDrag),
					label: String::from("Drag Selected"),
					plus: false,
				}]),
				HintGroup(vec![
					HintInfo {
						key_groups: vec![
							KeysGroup(vec![Key::KeyArrowUp]),
							KeysGroup(vec![Key::KeyArrowRight]),
							KeysGroup(vec![Key::KeyArrowDown]),
							KeysGroup(vec![Key::KeyArrowLeft]),
						],
						mouse: None,
						label: String::from("Nudge Selected (coming soon)"),
						plus: false,
					},
					HintInfo {
						key_groups: vec![KeysGroup(vec![Key::KeyShift])],
						mouse: None,
						label: String::from("Big Increment Nudge"),
						plus: true,
					},
				]),
				HintGroup(vec![
					HintInfo {
						key_groups: vec![KeysGroup(vec![Key::KeyG])],
						mouse: None,
						label: String::from("Grab Selected (coming soon)"),
						plus: false,
					},
					HintInfo {
						key_groups: vec![KeysGroup(vec![Key::KeyR])],
						mouse: None,
						label: String::from("Rotate Selected (coming soon)"),
						plus: false,
					},
					HintInfo {
						key_groups: vec![KeysGroup(vec![Key::KeyS])],
						mouse: None,
						label: String::from("Scale Selected (coming soon)"),
						plus: false,
					},
				]),
			]),
			PathToolFsmState::Dragging => HintData(vec![HintGroup(vec![HintInfo {
				key_groups: vec![KeysGroup(vec![Key::KeyAlt])],
				mouse: None,
				label: String::from("Handle Mirroring Toggle"),
				plus: false,
			}])]),
		};

		responses.push_back(FrontendMessage::UpdateInputHints { hint_data }.into());
	}

	fn update_cursor(&self, responses: &mut VecDeque<Message>) {
		responses.push_back(FrontendMessage::UpdateMouseCursor { cursor: MouseCursorIcon::Default }.into());
	}
}

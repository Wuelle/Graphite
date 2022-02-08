use crate::consts::SELECTION_TOLERANCE;
use crate::document::DocumentMessageHandler;
use crate::frontend::utility_types::MouseCursorIcon;
use crate::input::keyboard::Key;
use crate::input::InputPreprocessorMessageHandler;
use crate::layout::widgets::PropertyHolder;
use crate::message_prelude::*;
use crate::viewport_tools::snapping::SnapHandler;
use crate::viewport_tools::tool::{DocumentToolData, Fsm, ToolActionHandlerData};

use graphene::intersection::Quad;

use super::shared::transformation_cage::*;

use glam::{DVec2, Vec2Swizzles};
use serde::{Deserialize, Serialize};

#[derive(Default)]
pub struct Crop {
	fsm_state: CropToolFsmState,
	data: CropToolData,
}

#[remain::sorted]
#[impl_message(Message, ToolMessage, Crop)]
#[derive(PartialEq, Clone, Debug, Hash, Serialize, Deserialize)]
pub enum CropMessage {
	// Standard messages
	#[remain::unsorted]
	Abort,
	#[remain::unsorted]
	DocumentIsDirty,

	// Tool-specific messages
	MouseDown,
	MouseMove {
		axis_align: Key,
		centre: Key,
	},
	MouseUp,
}

impl<'a> MessageHandler<ToolMessage, ToolActionHandlerData<'a>> for Crop {
	fn process_action(&mut self, action: ToolMessage, data: ToolActionHandlerData<'a>, responses: &mut VecDeque<Message>) {
		if action == ToolMessage::UpdateHints {
			self.fsm_state.update_hints(responses);
			return;
		}

		if action == ToolMessage::UpdateCursor {
			responses.push_back(FrontendMessage::UpdateMouseCursor { cursor: MouseCursorIcon::Default }.into());
			return;
		}

		let new_state = self.fsm_state.transition(action, data.0, data.1, &mut self.data, &(), data.2, responses);

		if self.fsm_state != new_state {
			self.fsm_state = new_state;
			self.fsm_state.update_hints(responses);
		}
	}

	advertise_actions!(CropMessageDiscriminant; MouseDown, MouseUp, MouseMove, Abort);
}

impl PropertyHolder for Crop {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CropToolFsmState {
	Ready,
	Drawing,
	ResizingBounds,
}

impl Default for CropToolFsmState {
	fn default() -> Self {
		CropToolFsmState::Ready
	}
}

#[derive(Clone, Debug, Default)]
struct CropToolData {
	bounding_box_overlays: Option<BoundingBoxOverlays>,
	selected_board: Option<Vec<LayerId>>,
	snap_handler: SnapHandler,
	cursor: MouseCursorIcon,
	drag_start: DVec2,
}

impl Fsm for CropToolFsmState {
	type ToolData = CropToolData;
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
		if let ToolMessage::Crop(event) = event {
			match (self, event) {
				(CropToolFsmState::Ready | CropToolFsmState::ResizingBounds, CropMessage::DocumentIsDirty) => {
					let mut buffer = Vec::new();
					match (
						data.selected_board.as_ref().map(|path| document.artboard_bounding_box_and_transform(&path)).unwrap_or(None),
						data.bounding_box_overlays.take(),
					) {
						(None, Some(bounding_box_overlays)) => bounding_box_overlays.delete(&mut buffer),
						(Some((bounds, transform)), paths) => {
							let mut bounding_box_overlays = paths.unwrap_or_else(|| BoundingBoxOverlays::new(&mut buffer));

							bounding_box_overlays.bounds = bounds;
							bounding_box_overlays.transform = transform;

							bounding_box_overlays.transform(&mut buffer);

							data.bounding_box_overlays = Some(bounding_box_overlays);

							responses.push_back(OverlaysMessage::Rerender.into());
						}
						(_, _) => {}
					};
					buffer.into_iter().rev().for_each(|message| responses.push_front(message));
					self
				}
				(CropToolFsmState::Ready, CropMessage::MouseDown) => {
					data.drag_start = input.mouse.position;

					let dragging_bounds = if let Some(bounding_box) = &mut data.bounding_box_overlays {
						let edges = bounding_box.check_selected_edges(input.mouse.position);

						bounding_box.selected_edges = edges.map(|(top, bottom, left, right)| {
							let edges = SelectedEdges::new(top, bottom, left, right, bounding_box.bounds);
							bounding_box.pivot = edges.calculate_pivot();
							edges
						});

						edges
					} else {
						None
					};

					if let Some(selected_edges) = dragging_bounds {
						let snap_x = selected_edges.2 || selected_edges.3;
						let snap_y = selected_edges.0 || selected_edges.1;

						data.snap_handler.start_snap(document, document.visible_layers(), snap_x, snap_y);

						CropToolFsmState::ResizingBounds
					} else {
						let tolerance = DVec2::splat(SELECTION_TOLERANCE);
						let quad = Quad::from_box([input.mouse.position - tolerance, input.mouse.position + tolerance]);
						let intersection = document.artboard_message_handler.artboards_graphene_document.intersects_quad_root(quad);

						responses.push_back(ToolMessage::DocumentIsDirty.into());
						if let Some(intersection) = intersection.last() {
							data.selected_board = Some(intersection.clone());

							CropToolFsmState::Ready
						} else {
							let id = generate_uuid();
							data.selected_board = Some(vec![id]);

							data.snap_handler.start_snap(document, document.visible_layers(), true, true);

							responses.push_back(
								ArtboardMessage::AddArtboard {
									id: Some(id),
									position: (0., 0.),
									size: (0., 0.),
								}
								.into(),
							);

							CropToolFsmState::Drawing
						}
					}
				}
				(CropToolFsmState::ResizingBounds, CropMessage::MouseMove { axis_align, centre }) => {
					if let Some(bounds) = &mut data.bounding_box_overlays {
						if let Some(movement) = &mut bounds.selected_edges {
							let (centre, axis_align) = (input.keyboard.get(centre as usize), input.keyboard.get(axis_align as usize));
							let mouse_position = input.mouse.position;

							let snapped_mouse_position = data.snap_handler.snap_position(responses, input.viewport_bounds.size(), document, mouse_position);

							let [position, size] = movement.new_size(snapped_mouse_position, bounds.transform, centre, axis_align);
							let position = movement.centre_position(position, size, centre);

							responses.push_back(
								ArtboardMessage::ResizeArtboard {
									artboard: data.selected_board.clone().unwrap(),
									position: position.into(),
									size: size.into(),
								}
								.into(),
							);

							responses.push_back(ToolMessage::DocumentIsDirty.into());
						}
					}
					CropToolFsmState::ResizingBounds
				}
				(CropToolFsmState::Drawing, CropMessage::MouseMove { axis_align, centre }) => {
					let mouse_position = input.mouse.position;
					let snapped_mouse_position = data.snap_handler.snap_position(responses, input.viewport_bounds.size(), document, mouse_position);

					let root_transform = document.graphene_document.root.transform.inverse();

					let mut start = data.drag_start;
					let mut size = snapped_mouse_position - start;
					if input.keyboard.get(axis_align as usize) {
						size = size.abs().max(size.abs().yx()) * size.signum();
					}
					if input.keyboard.get(centre as usize) {
						start -= size;
						size *= 2.;
					}

					let start = root_transform.transform_point2(start);
					let size = root_transform.transform_vector2(size);

					responses.push_back(
						ArtboardMessage::ResizeArtboard {
							artboard: data.selected_board.clone().unwrap(),
							position: start.into(),
							size: size.into(),
						}
						.into(),
					);

					responses.push_back(ToolMessage::DocumentIsDirty.into());

					CropToolFsmState::Drawing
				}
				(CropToolFsmState::Ready, CropMessage::MouseMove { .. }) => {
					let cursor = data.bounding_box_overlays.as_ref().map_or(MouseCursorIcon::Default, |bounds| bounds.get_cursor(input));

					if data.cursor != cursor {
						data.cursor = cursor;
						responses.push_back(FrontendMessage::UpdateMouseCursor { cursor }.into());
					}

					CropToolFsmState::Ready
				}
				(CropToolFsmState::ResizingBounds, CropMessage::MouseUp) => {
					data.snap_handler.cleanup(responses);

					if let Some(bounds) = &mut data.bounding_box_overlays {
						bounds.original_transforms.clear();
					}

					CropToolFsmState::Ready
				}
				(CropToolFsmState::Drawing, CropMessage::MouseUp) => {
					data.snap_handler.cleanup(responses);

					if let Some(bounds) = &mut data.bounding_box_overlays {
						bounds.original_transforms.clear();
					}

					responses.push_back(ToolMessage::DocumentIsDirty.into());

					CropToolFsmState::Ready
				}
				(_, CropMessage::Abort) => {
					if let Some(bounding_box_overlays) = data.bounding_box_overlays.take() {
						bounding_box_overlays.delete(responses);
					}

					data.snap_handler.cleanup(responses);
					CropToolFsmState::Ready
				}
				_ => self,
			}
		} else {
			self
		}
	}

	fn update_hints(&self, _responses: &mut VecDeque<Message>) {}

	fn update_cursor(&self, responses: &mut VecDeque<Message>) {
		responses.push_back(FrontendMessage::UpdateMouseCursor { cursor: MouseCursorIcon::Default }.into());
	}
}

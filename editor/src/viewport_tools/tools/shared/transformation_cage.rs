use crate::consts::{BOUNDS_ROTATE_THRESHOLD, BOUNDS_SELECT_THRESHOLD, COLOR_ACCENT, VECTOR_MANIPULATOR_ANCHOR_MARKER_SIZE};
use crate::document::transformation::OriginalTransforms;
use crate::frontend::utility_types::MouseCursorIcon;
use crate::input::InputPreprocessorMessageHandler;
use crate::message_prelude::*;

use graphene::color::Color;
use graphene::layers::style::{self, Fill, Stroke};
use graphene::Operation;

use glam::{DAffine2, DVec2, Vec2Swizzles};

#[derive(Clone, Debug, Default)]
pub struct SelectedEdges {
	bounds: [DVec2; 2],
	top: bool,
	bottom: bool,
	left: bool,
	right: bool,
}

impl SelectedEdges {
	pub fn new(top: bool, bottom: bool, left: bool, right: bool, bounds: [DVec2; 2]) -> Self {
		Self { top, bottom, left, right, bounds }
	}

	/// Calculate the pivot for the operation (the opposite point to the edge dragged)
	pub fn calculate_pivot(&self) -> DVec2 {
		let min = self.bounds[0];
		let max = self.bounds[1];

		let x = if self.left {
			max.x
		} else if self.right {
			min.x
		} else {
			(min.x + max.x) / 2.
		};

		let y = if self.top {
			max.y
		} else if self.bottom {
			min.y
		} else {
			(min.y + max.y) / 2.
		};

		DVec2::new(x, y)
	}

	/// Computes the new bounds with the given mouse move and modifier keys
	pub fn new_size(&self, mouse: DVec2, centre: bool, constrain: bool) -> [DVec2; 2] {
		let mut min = self.bounds[0];
		let mut max = self.bounds[1];
		if self.top {
			min.y = mouse.y;
		} else if self.bottom {
			max.y = mouse.y;
		}
		if self.left {
			min.x = mouse.x
		} else if self.right {
			max.x = mouse.x;
		}

		let mut size = max - min;
		if constrain && ((self.top || self.bottom) && (self.left || self.right)) {
			size = size.abs().max(size.abs().yx()) * size.signum();
		}
		if centre {
			if self.left || self.right {
				size.x *= 2.;
			}

			if self.bottom || self.top {
				size.y *= 2.;
			}
		}

		[min, size]
	}

	/// Offsets the transformation pivot in order to scale from the centre
	fn offset_pivot(&self, centre: bool, size: DVec2) -> DVec2 {
		let mut offset = DVec2::ZERO;

		if centre && self.right {
			offset.x -= size.x / 2.;
		}
		if centre && self.left {
			offset.x += size.x / 2.;
		}
		if centre && self.bottom {
			offset.y -= size.y / 2.;
		}
		if centre && self.top {
			offset.y += size.y / 2.;
		}
		offset
	}

	pub fn centre_position(&self, mut position: DVec2, size: DVec2, centre: bool) -> DVec2 {
		if centre && self.right {
			position.x -= size.x / 2.;
		}
		if centre && self.bottom {
			position.y -= size.y / 2.;
		}

		position
	}

	/// Calculates the required scaling to resize the bounding box
	pub fn bounds_to_scale_transform(&self, centre: bool, min: DVec2, size: DVec2) -> DAffine2 {
		let translation = DAffine2::from_translation(self.offset_pivot(centre, size));
		translation * DAffine2::from_scale(size / (self.bounds[1] - self.bounds[0]))
	}
}

pub fn add_bounding_box(responses: &mut Vec<Message>) -> Vec<LayerId> {
	let path = vec![generate_uuid()];

	let operation = Operation::AddOverlayRect {
		path: path.clone(),
		transform: DAffine2::ZERO.to_cols_array(),
		style: style::PathStyle::new(Some(Stroke::new(COLOR_ACCENT, 1.0)), None),
	};
	responses.push(DocumentMessage::Overlays(operation.into()).into());

	path
}

fn evaluate_transform_handle_positions((left, top): (f64, f64), (right, bottom): (f64, f64)) -> [DVec2; 8] {
	[
		DVec2::new(left, top),
		DVec2::new(left, (top + bottom) / 2.),
		DVec2::new(left, bottom),
		DVec2::new((left + right) / 2., top),
		DVec2::new((left + right) / 2., bottom),
		DVec2::new(right, top),
		DVec2::new(right, (top + bottom) / 2.),
		DVec2::new(right, bottom),
	]
}

fn add_transform_handles(responses: &mut Vec<Message>) -> [Vec<LayerId>; 8] {
	const EMPTY_VEC: Vec<LayerId> = Vec::new();
	let mut transform_handle_paths = [EMPTY_VEC; 8];

	for item in &mut transform_handle_paths {
		let current_path = vec![generate_uuid()];

		let operation = Operation::AddOverlayRect {
			path: current_path.clone(),
			transform: DAffine2::ZERO.to_cols_array(),
			style: style::PathStyle::new(Some(Stroke::new(COLOR_ACCENT, 2.0)), Some(Fill::new(Color::WHITE))),
		};
		responses.push(DocumentMessage::Overlays(operation.into()).into());

		*item = current_path;
	}

	transform_handle_paths
}

pub fn transform_from_box(pos1: DVec2, pos2: DVec2) -> [f64; 6] {
	DAffine2::from_scale_angle_translation((pos2 - pos1).round(), 0., pos1.round() - DVec2::splat(0.5)).to_cols_array()
}

/// Contains info on the overlays for the bounding box and transform handles
#[derive(Clone, Debug, Default)]
pub struct BoundingBoxOverlays {
	pub bounding_box: Vec<LayerId>,
	pub transform_handles: [Vec<LayerId>; 8],
	pub bounds: [DVec2; 2],
	pub selected_edges: Option<SelectedEdges>,
	pub original_transforms: OriginalTransforms,
	pub pivot: DVec2,
}

impl BoundingBoxOverlays {
	#[must_use]
	pub fn new(buffer: &mut Vec<Message>) -> Self {
		Self {
			bounding_box: add_bounding_box(buffer),
			transform_handles: add_transform_handles(buffer),
			..Default::default()
		}
	}

	/// Update the position of the bounding box and transform handles
	pub fn transform(&mut self, buffer: &mut Vec<Message>) {
		let transform = transform_from_box(self.bounds[0], self.bounds[1]);
		let path = self.bounding_box.clone();
		buffer.push(DocumentMessage::Overlays(Operation::SetLayerTransformInViewport { path, transform }.into()).into());

		// Helps push values that end in approximately half, plus or minus some floating point imprecision, towards the same side of the round() function
		const BIAS: f64 = 0.0001;

		for (position, path) in evaluate_transform_handle_positions(self.bounds[0].into(), self.bounds[1].into())
			.into_iter()
			.zip(&self.transform_handles)
		{
			let scale = DVec2::splat(VECTOR_MANIPULATOR_ANCHOR_MARKER_SIZE);
			let translation = (position - (scale / 2.) - 0.5 + BIAS).round();
			let transform = DAffine2::from_scale_angle_translation(scale, 0., translation).to_cols_array();
			let path = path.clone();
			buffer.push(DocumentMessage::Overlays(Operation::SetLayerTransformInViewport { path, transform }.into()).into());
		}
	}

	/// Check if the user has selected the edge for dragging (returns which edge in order top, bottom, left, right)
	pub fn check_selected_edges(&self, cursor: DVec2) -> Option<(bool, bool, bool, bool)> {
		let min = self.bounds[0].min(self.bounds[1]);
		let max = self.bounds[0].max(self.bounds[1]);
		if min.x - cursor.x < BOUNDS_SELECT_THRESHOLD && min.y - cursor.y < BOUNDS_SELECT_THRESHOLD && cursor.x - max.x < BOUNDS_SELECT_THRESHOLD && cursor.y - max.y < BOUNDS_SELECT_THRESHOLD {
			let mut top = (cursor.y - min.y).abs() < BOUNDS_SELECT_THRESHOLD;
			let mut bottom = (max.y - cursor.y).abs() < BOUNDS_SELECT_THRESHOLD;
			let mut left = (cursor.x - min.x).abs() < BOUNDS_SELECT_THRESHOLD;
			let mut right = (max.x - cursor.x).abs() < BOUNDS_SELECT_THRESHOLD;
			if cursor.y - min.y + max.y - cursor.y < BOUNDS_SELECT_THRESHOLD * 2. && (left || right) {
				top = false;
				bottom = false;
			}
			if cursor.x - min.x + max.x - cursor.x < BOUNDS_SELECT_THRESHOLD * 2. && (top || bottom) {
				left = false;
				right = false;
			}

			if top || bottom || left || right {
				return Some((top, bottom, left, right));
			}
		}

		None
	}

	/// Check if the user is rotating with the bounds
	pub fn check_rotate(&self, cursor: DVec2) -> bool {
		let min = self.bounds[0].min(self.bounds[1]);
		let max = self.bounds[0].max(self.bounds[1]);

		let outside_bounds = (min.x > cursor.x || cursor.x > max.x) || (min.y > cursor.y || cursor.y > max.y);
		let inside_extended_bounds =
			min.x - cursor.x < BOUNDS_ROTATE_THRESHOLD && min.y - cursor.y < BOUNDS_ROTATE_THRESHOLD && cursor.x - max.x < BOUNDS_ROTATE_THRESHOLD && cursor.y - max.y < BOUNDS_ROTATE_THRESHOLD;

		outside_bounds & inside_extended_bounds
	}

	pub fn get_cursor(&self, input: &InputPreprocessorMessageHandler) -> MouseCursorIcon {
		if let Some(directions) = self.check_selected_edges(input.mouse.position) {
			match directions {
				(true, false, false, false) | (false, true, false, false) => MouseCursorIcon::NSResize,
				(false, false, true, false) | (false, false, false, true) => MouseCursorIcon::EWResize,
				(true, false, true, false) | (false, true, false, true) => MouseCursorIcon::NWSEResize,
				(true, false, false, true) | (false, true, true, false) => MouseCursorIcon::NESWResize,
				_ => MouseCursorIcon::Default,
			}
		} else if self.check_rotate(input.mouse.position) {
			MouseCursorIcon::Grabbing
		} else {
			MouseCursorIcon::Default
		}
	}

	/// Removes the overlays
	pub fn delete(self, buffer: &mut impl Extend<Message>) {
		buffer.extend([DocumentMessage::Overlays(Operation::DeleteLayer { path: self.bounding_box }.into()).into()]);
		buffer.extend(
			self.transform_handles
				.iter()
				.map(|path| DocumentMessage::Overlays(Operation::DeleteLayer { path: path.clone() }.into()).into()),
		);
	}
}

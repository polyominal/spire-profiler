extends Control

# Replays only commands exported by the original Rust implementation.
var fixture: Dictionary
var contract: Dictionary
var context: Dictionary
var clip: Control
var body: Control
var overlay: Control
var fonts: Dictionary = {}
var textures: Dictionary = {}
var plate: StyleBoxTexture
var shadow: StyleBoxTexture

func configure(fixture_json: String, contract_json: String) -> void:
	fixture = JSON.parse_string(fixture_json)
	contract = JSON.parse_string(contract_json)
	context = fixture.render
	mouse_filter = Control.MOUSE_FILTER_IGNORE
	size = vec(context.viewport)
	for role in contract.fonts:
		fonts[role] = load(contract.fonts[role])
	var skin = contract.plate
	plate = StyleBoxTexture.new()
	plate.texture = load(skin.path)
	plate.region_rect = Rect2(skin.region[0], skin.region[1], skin.region[2], skin.region[3])
	plate.texture_margin_left = skin.margins[0]
	plate.texture_margin_top = skin.margins[1]
	plate.texture_margin_right = skin.margins[2]
	plate.texture_margin_bottom = skin.margins[3]
	plate.axis_stretch_horizontal = StyleBoxTexture.AXIS_STRETCH_MODE_TILE
	plate.axis_stretch_vertical = StyleBoxTexture.AXIS_STRETCH_MODE_TILE
	shadow = plate.duplicate()
	shadow.modulate_color = rgba(skin.shadow)
	clip = Control.new()
	clip.mouse_filter = Control.MOUSE_FILTER_IGNORE
	clip.position = rect(context.control).position
	clip.size = rect(context.control).size
	clip.clip_contents = true
	clip.draw.connect(draw_header)
	add_child(clip)
	body = Control.new()
	body.mouse_filter = Control.MOUSE_FILTER_IGNORE
	body.position = rect(context.body_clip).position - clip.position
	body.size = rect(context.body_clip).size
	body.clip_contents = true
	body.draw.connect(draw_body)
	clip.add_child(body)
	overlay = Control.new()
	overlay.mouse_filter = Control.MOUSE_FILTER_IGNORE
	overlay.size = clip.size
	overlay.draw.connect(draw_overlays)
	clip.add_child(overlay)
	queue_redraw()

func _draw() -> void:
	if not contract.is_empty():
		draw_rect(Rect2(Vector2.ZERO, size), rgba(contract.backdrop))

func vec(value: Array) -> Vector2:
	return Vector2(value[0], value[1])

func rect(value: Dictionary) -> Rect2:
	return Rect2(value.x, value.y, value.w, value.h)

func rgba(value: Array) -> Color:
	return Color(value[0], value[1], value[2], value[3])

func asset(path: String) -> Texture2D:
	if not textures.has(path):
		textures[path] = load(path)
	return textures[path]

func font_for(canvas: Control, role: String) -> Font:
	if context.font_plan == "Fallback" or fonts[role] == null:
		return canvas.get_theme_default_font()
	return fonts[role]

func skin(canvas: Control, box: Rect2) -> void:
	if context.flat_chrome:
		canvas.draw_rect(box, rgba(contract.panel_background))
		var color = rgba(contract.panel_border)
		canvas.draw_rect(Rect2(box.position, Vector2(box.size.x, 1)), color)
		canvas.draw_rect(Rect2(box.position + Vector2(0, box.size.y - 1), Vector2(box.size.x, 1)), color)
		canvas.draw_rect(Rect2(box.position, Vector2(1, box.size.y)), color)
		canvas.draw_rect(Rect2(box.position + Vector2(box.size.x - 1, 0), Vector2(1, box.size.y)), color)
	else:
		canvas.draw_style_box(shadow, Rect2(box.position + Vector2(8, 8), box.size - Vector2(8, 8)))
		canvas.draw_style_box(plate, Rect2(box.position, box.size - Vector2(8, 8)))

func draw_header() -> void:
	var box = rect(context.plate)
	box.position -= clip.position
	if context.flat_chrome:
		clip.draw_rect(box, rgba(contract.panel_background))
		if not fixture.history:
			clip.draw_rect(Rect2(vec(context.header_offset) - clip.position, Vector2(context.box_size[0], fixture.expected.header_bottom)), rgba(contract.panel_background))
	else:
		skin(clip, box)
	replay(clip, fixture.expected.header, vec(context.header_offset) - clip.position)

func draw_body() -> void:
	replay(body, fixture.expected.body, vec(context.body_offset) - rect(context.body_clip).position)

func replay(canvas: Control, commands: Array, offset: Vector2) -> void:
	for command in commands:
		var pos = Vector2(command.x, command.y) + offset
		match command.type:
			"rect":
				canvas.draw_rect(Rect2(pos, Vector2(command.w, command.h)), rgba(command.color))
			"text":
				var alignment = HORIZONTAL_ALIGNMENT_LEFT
				if command.align.kind == "Right": alignment = HORIZONTAL_ALIGNMENT_RIGHT
				if command.align.kind == "Center": alignment = HORIZONTAL_ALIGNMENT_CENTER
				var width = command.align.get("width", -1)
				for effect in contract.text_effects[command.effect]:
					var color = rgba(command.color) if effect.color is String else rgba(effect.color)
					canvas.draw_string(font_for(canvas, command.role), pos + vec(effect.offset), command.text, alignment, width, command.size, color)
			"texture":
				var texture: Texture2D
				var color = Color.WHITE
				var texture_scale = 1.0
				if command.icon == "Character":
					texture = asset(fixture.expected.portrait_paths[int(command.slot)])
					texture_scale = context.avatar_scales[int(command.slot)]
					if context.player != null and context.player != context.avatar_slots[int(command.slot)]: color = rgba(contract.avatar_dim_modulate)
				else:
					texture = asset(contract.textures[command.icon].path)
					color = rgba(contract.textures[command.icon].modulate)
				var dimensions = Vector2(command.w, command.h)
				canvas.draw_texture_rect(texture, Rect2(pos - dimensions * (texture_scale - 1) / 2, dimensions * texture_scale), false, color)

func draw_overlays() -> void:
	var offset = -clip.position
	if context.scrollbar != null:
		for pair in [["ScrollCenter", "body"], ["ScrollEdge", "cap_top"], ["ScrollEdge", "cap_bottom"], ["ScrollTrain", "grabber"]]:
			var box = rect(context.scrollbar[pair[1]])
			box.position += offset
			var color = Color.WHITE if pair[0] == "ScrollTrain" else rgba(contract.scroll_track_modulate)
			overlay.draw_texture_rect(asset(contract.textures[pair[0]]), box, false, color)
	if context.legend != null:
		var box = rect(context.legend)
		box.position += offset
		skin(overlay, box)
		replay(overlay, context.legend_commands, offset)
	if context.tip != null:
		var box = rect(context.tip)
		box.position += offset
		skin(overlay, box)
		var style = contract.tooltip
		var y = box.position.y + style.first_baseline
		for line in context.tip_lines:
			var font = font_for(overlay, "Title" if line.title else "Body")
			var pos = Vector2(box.position.x + style.text_x, y)
			overlay.draw_string(font, pos + vec(style.shadow_offset), line.text, HORIZONTAL_ALIGNMENT_LEFT, -1, style.size, rgba(style.shadow))
			overlay.draw_string(font, pos, line.text, HORIZONTAL_ALIGNMENT_LEFT, -1, style.size, rgba(line.color))
			if line.value != null:
				pos.x = box.position.x + style.value_x
				overlay.draw_string(font, pos + vec(style.shadow_offset), line.value.text, HORIZONTAL_ALIGNMENT_RIGHT, style.value_width, style.size, rgba(style.shadow))
				overlay.draw_string(font, pos, line.value.text, HORIZONTAL_ALIGNMENT_RIGHT, style.value_width, style.size, rgba(line.value.color))
			y += style.line_height

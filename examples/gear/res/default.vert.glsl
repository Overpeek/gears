#version 420
#extension GL_ARB_separate_shader_objects : enable

#[gears_bindgen(uniform)]
struct UBO {
	mat4 model_matrix;
	mat4 view_matrix;
	mat4 projection_matrix;
	vec3 light_dir;
} ubo;

#[gears_bindgen(in)]
struct VertexData {
	vec3 pos;
	vec3 norm;
} vert_in;

#[gears_gen(out)]
struct VFSharedData {
	float exposure;
} vert_out;



void main() {
	mat4 mvp = ubo.projection_matrix * ubo.view_matrix * ubo.model_matrix;
	gl_Position = mvp * vec4(vert_in.pos, 1.0);
	vec3 normal = (ubo.model_matrix * vec4(vert_in.norm, 1.0)).xyz;

	vert_out.exposure = 1.0 + dot(normal, ubo.light_dir);
}
#version 420
#extension GL_ARB_separate_shader_objects : enable

#[gears_gen(in)]
struct VFSharedData {
	float exposure;
} frag_in;

#[gears_gen(out)]
struct VFFragmentData {
	vec4 col;
} frag_out;



void main() {
	vec3 color = vec3(1.0) * smoothstep(0.1, 1.9, frag_in.exposure);
	frag_out.col = vec4(color, 1.0);
}
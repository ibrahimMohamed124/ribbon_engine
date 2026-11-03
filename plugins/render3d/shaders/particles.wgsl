struct Globals { view_proj: mat4x4<f32>, }
@group(0) @binding(0) var<uniform> globals: Globals;
struct CameraBasis { right: vec4<f32>, up: vec4<f32>, }
@group(1) @binding(0) var<uniform> camera: CameraBasis;
struct In {
 @location(0) position: vec3<f32>, @location(1) rotation: f32,
 @location(2) velocity: vec3<f32>, @location(3) emissive: f32,
 @location(4) size: vec2<f32>, @location(5) shape_billboard: vec2<f32>,
 @location(6) color: vec4<f32>,
}
struct Out { @builtin(position) clip: vec4<f32>, @location(0) local: vec2<f32>, @location(1) color: vec4<f32>, @location(2) shape: f32, @location(3) emissive: f32, }
@vertex fn vs_main(input: In, @builtin(vertex_index) vertex: u32) -> Out {
 var corners = array<vec2<f32>,6>(vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,-1.0),vec2<f32>(1.0,1.0),vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,1.0),vec2<f32>(-1.0,1.0));
 let c=corners[vertex]; let right=normalize(camera.right.xyz); let up=normalize(camera.up.xyz);
 var angle=input.rotation; var width=input.size.x; var height=input.size.y;
 if input.shape_billboard.y>=0.0 { let speed=length(input.velocity); height=height+speed*input.shape_billboard.y; let projected=right*dot(input.velocity,right)+up*dot(input.velocity,up); if length(projected)>0.00001 { angle=atan2(dot(projected,up),dot(projected,right)); } }
 let ca=cos(angle); let sa=sin(angle); let r=right*ca+up*sa; let u=up*ca-right*sa;
 let world=input.position+r*(c.x*width*0.5)+u*(c.y*height*0.5);
 var out:Out; out.clip=globals.view_proj*vec4<f32>(world,1.0);out.local=c;out.color=input.color;out.shape=input.shape_billboard.x;out.emissive=input.emissive;return out;
}
@fragment fn fs_main(input:Out)->@location(0) vec4<f32>{var mask=1.0;if input.shape<0.5 {let d=length(input.local);mask=1.0-smoothstep(0.65,1.0,d);}else if input.shape<1.5 {let edge=max(abs(input.local.x),abs(input.local.y));mask=(1.0-smoothstep(0.75,1.0,edge))*(0.65+0.35*(1.0-abs(input.local.x)));}let alpha=input.color.a*mask;return vec4<f32>(input.color.rgb*(1.0+input.emissive),alpha);}

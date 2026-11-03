struct VOut{@builtin(position) pos:vec4<f32>,@location(0) color:vec4<f32>};
@vertex fn vs_main(@location(0) p:vec2<f32>,@location(1) c:vec4<f32>)->VOut{var o:VOut;o.pos=vec4<f32>(p.x*2.0-1.0,1.0-p.y*2.0,0.0,1.0);o.color=c;return o;}
@fragment fn fs_main(i:VOut)->@location(0) vec4<f32>{return i.color;}

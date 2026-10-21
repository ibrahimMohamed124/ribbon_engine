const PBR_MAX_DIMENSION: u32 = 256u;
const PBR_MIP_TEXEL_COUNT: u32 = 87381u;
const PBR_MAPS_PER_SET: u32 = 4u;
const PBR_MIP_LEVEL_COUNT: u32 = 9u;
struct WorldUniform { dims: vec4<u32>, eye: vec4<f32>, forward: vec4<f32>, right: vec4<f32>, up: vec4<f32>, projection: vec4<f32>, lighting: vec4<f32>, style: vec4<f32>, frame: vec4<u32>, sprite: vec4<u32>, sprite_meta: vec4<u32>, entity_limits: vec4<u32> }
struct Material { info: vec4<u32>, params: vec4<f32> }
struct Entity { pos_yaw: vec4<f32>, params: vec4<f32>, bounds_min: vec4<f32>, bounds_max: vec4<f32>, range: vec4<u32>, animation: vec4<f32>, render: vec4<f32>, shadow: vec4<f32> }
struct Box { min_part: vec4<f32>, max_sway: vec4<f32> }
@group(0) @binding(0) var<uniform> u: WorldUniform;
@group(0) @binding(1) var<storage, read> voxels: array<u32>;
@group(0) @binding(2) var<storage, read> materials: array<Material>;
@group(0) @binding(3) var<storage, read> entities: array<Entity>;
@group(0) @binding(4) var<storage, read> boxes: array<Box>;
@group(0) @binding(5) var<storage, read> tone: array<f32>;
@group(0) @binding(6) var<storage, read> gamma: array<f32>;
@group(0) @binding(7) var<storage, read> palette: array<u32>;
@group(0) @binding(8) var brick: texture_2d<f32>;
@group(0) @binding(9) var noise_tex: texture_2d<f32>;
@group(0) @binding(10) var<storage, read> pbr_texels: array<u32>;
struct Hit { ok: bool, id: u32, cell: vec3<i32>, normal: vec3<i32>, point: vec3<f32>, t: f32 }
struct BoxHit { ok: bool, t: f32, axis: u32, sign: f32 }
struct EntityHit { ok: bool, t: f32, axis: u32, sign: f32, part: u32, kind: u32, point: vec3<f32>, ndotl: f32 }
struct SpriteHit { ok: bool, t: f32, u: f32, v: f32, view: u32, state: u32, frame: u32 }
@vertex fn vs_main(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32>{let p=array<vec2<f32>,3>(vec2<f32>(-1.,-1.),vec2<f32>(3.,-1.),vec2<f32>(-1.,3.));return vec4<f32>(p[i],0.,1.);}
fn voxel(c:vec3<i32>)->i32{if(any(c<vec3<i32>(0))||any(c>=vec3<i32>(u.dims.xyz))){return -1;}let i=(u32(c.z)*u.dims.y+u32(c.y))*u.dims.x+u32(c.x);return i32((voxels[i>>2u]>>((i&3u)*8u))&255u);}
fn cast_world(o:vec3<f32>,d:vec3<f32>,max_t:f32)->Hit{var c=vec3<i32>(floor(o));var st=vec3<i32>(0);var delta=vec3<f32>(1e30);var edge=vec3<f32>(1e30);for(var a=0u;a<3u;a++){if(d[a]>0.){st[a]=1;delta[a]=1./d[a];edge[a]=(f32(c[a]+1)-o[a])*delta[a];}else if(d[a]<0.){st[a]=-1;delta[a]=-1./d[a];edge[a]=(o[a]-f32(c[a]))*delta[a];}}var n=vec3<i32>(0);var t=0.;for(var k=0u;k<58u;k++){let id=voxel(c);if(id<0){break;}if(u32(id)!=u.frame.w){return Hit(true,u32(id),c,n,o+d*t,t);}var a=2u;if(edge.x<edge.y){if(edge.x<edge.z){a=0u;}}else if(edge.y<edge.z){a=1u;}if(edge[a]>max_t){break;}t=edge[a];c[a]+=st[a];edge[a]+=delta[a];n=vec3<i32>(0);n[a]=-st[a];}return Hit(false,0u,vec3<i32>(0),vec3<i32>(0),vec3<f32>(0.),1e30);}
fn shadow_cone(h:Hit,cone:f32)->f32{if((u32(u.projection.w)&1u)==0u||u.lighting.x<=0.||cone<=0.){return cone;}let to_light=normalize(u.eye.xyz-h.point);let origin=h.point+vec3<f32>(h.normal)*0.02;let blocker=cast_world(origin,to_light,length(u.eye.xyz-h.point));return select(cone,0.,blocker.ok);}
fn box_hit(o:vec3<f32>,d:vec3<f32>,mn:vec3<f32>,mx:vec3<f32>)->BoxHit{var lo=-1e30;var hi=1e30;var ax=0u;var sg=1.;for(var a=0u;a<3u;a++){if(abs(d[a])<1e-8){if(o[a]<mn[a]||o[a]>mx[a]){return BoxHit(false,0.,0u,0.);}}else{var x=(mn[a]-o[a])/d[a];var y=(mx[a]-o[a])/d[a];var s=-1.;if(x>y){let z=x;x=y;y=z;s=1.;}if(x>lo){lo=x;ax=a;sg=s;}hi=min(hi,y);if(lo>hi){return BoxHit(false,0.,0u,0.);}}}return BoxHit(lo>=0.,lo,ax,sg);}
fn rotate(v:vec3<f32>,c:f32,s:f32)->vec3<f32>{return vec3<f32>(c*v.x+s*v.y,-s*v.x+c*v.y,v.z);}
fn animated_transform(e:Entity)->vec4<f32>{var t=e.pos_yaw;if(e.animation.x>0.5){let phase=e.params.x*e.animation.z+e.params.y;t.z+=sin(phase)*e.animation.y;t.w=e.params.x*e.animation.w+e.params.y;}return t;}
fn cast_entities(o:vec3<f32>,d:vec3<f32>,max_t:f32)->EntityHit{var best=EntityHit(false,max_t,0u,0.,0u,0u,vec3<f32>(0.),0.);for(var ei=0u;ei<min(u.frame.y,u.entity_limits.x);ei++){let e=entities[ei];let tr=animated_transform(e);if(u.style.w>0.5&&u.sprite.x>0u&&e.render.x>0.5){continue;}let c=cos(tr.w);let s=sin(tr.w);let lo=rotate(o-tr.xyz,c,s);let ld=rotate(d,c,s);if(e.range.z==1u){let bh=box_hit(lo,ld,e.bounds_min.xyz,e.bounds_max.xyz);if(bh.ok==false&&(any(lo<e.bounds_min.xyz)||any(lo>e.bounds_max.xyz))){continue;}}for(var bi=0u;bi<min(e.range.y,u.entity_limits.y);bi++){let b=boxes[e.range.x+bi];let sway=b.max_sway.w*sin(e.params.x)*.28;let mn=b.min_part.xyz+vec3<f32>(sway,0.,0.);let mx=b.max_sway.xyz+vec3<f32>(sway,0.,0.);let h=box_hit(lo,ld,mn,mx);if(h.ok&&h.t<best.t){best=EntityHit(true,h.t,h.axis,h.sign,u32(b.min_part.w),e.range.w,lo+ld*h.t,abs(ld[h.axis]));}}}return best;}
fn cast_sprites(o:vec3<f32>,d:vec3<f32>,max_t:f32)->SpriteHit{
 var best=SpriteHit(false,max_t,0.,0.,0u,0u,0u);
 if(u.sprite.x==0u||u.sprite.y==0u){return best;}
 for(var i=0u;i<min(u.frame.y,u.entity_limits.x);i++){
  let e=entities[i]; if(e.render.x<0.5){continue;}let tr=animated_transform(e);
  let rel=o.xy-tr.xy; let dist=length(rel); if(dist<.001){continue;}
  let toward=rel/dist; let denom=dot(d.xy,toward); if(denom>=-1e-6){continue;}
  let t=-dist/denom; if(t<=0.||t>=best.t){continue;}
  let p=o+d*t; let right=vec2<f32>(-toward.y,toward.x); let local=p.xy-tr.xy;
  let su=.5+dot(local,right)/max(e.render.z,0.001); let sv=(p.z-tr.z)/max(e.render.w,0.001);
  if(su<0.||su>=1.||sv<0.||sv>=1.){continue;}
  let angle=(atan2(rel.y,rel.x)-tr.w+6.28318530718)%6.28318530718;
  let views=max(u.sprite.x/u.sprite.z,1u);let view=min(u32(floor(angle/6.28318530718*f32(views))),views-1u);
  let states=max(u.sprite_meta.x,1u);let frames=max(u.sprite_meta.y,1u);
  let state=min(u32(max(round(e.params.y),0.)),states-1u);
  let frame=u32(floor(abs(e.params.x)/6.28318530718*f32(frames)))%frames;
  best=SpriteHit(true,t,su,sv,view,state,frame);
 }
 return best;
}
fn sprite_fetch(map:u32,hit:SpriteHit)->vec4<f32>{
 let x=min(u32(hit.u*f32(u.sprite.z)),u.sprite.z-1u); let y=min(u32((1.-hit.v)*f32(u.sprite.w)),u.sprite.w-1u);
 let row=hit.state*u.sprite_meta.y+hit.frame; let idx=(row*u.sprite.w+y)*u.sprite.x+hit.view*u.sprite.z+x;
 let word=pbr_texels[u.sprite_meta.z+map*u.sprite_meta.w+idx];
 return vec4<f32>(f32(word&255u),f32((word>>8u)&255u),f32((word>>16u)&255u),f32((word>>24u)&255u))/255.;
}
fn hash4(a:i32,b:i32,c:i32,d:i32)->f32{var h=bitcast<u32>(a)*73856093u^bitcast<u32>(b)*19349663u^bitcast<u32>(c)*83492791u^bitcast<u32>(d)*2654435761u;h^=h>>13u;h*=0x5bd1e995u;h^=h>>15u;return f32(h&65535u)/32767.5-1.;}
fn light(t:f32,nd:f32,cone:f32)->f32{let fog=clamp(1.-t/max(u.lighting.w,.001),0.,1.);let lamp=u.lighting.z/(1.+.7*t*t);let beam=u.lighting.x*cone*(.3+.7*nd)*(1.7/(1.+.06*t*t));return (u.lighting.y+lamp+beam)*fog;}
fn texel_fade(tex:texture_2d<f32>,uv:vec2<f32>,t:f32,fade0:f32,fade1:f32,avg:f32)->f32{let sz=vec2<i32>(textureDimensions(tex));let p=min(vec2<i32>(floor(fract(uv)*vec2<f32>(sz))),sz-1);return mix(textureLoad(tex,p,0).x,avg,clamp((t-fade0)/(fade1-fade0),0.,1.));}
fn decode_srgb(c:f32)->f32{if(c<=.04045){return c/12.92;}return pow((c+.055)/1.055,2.4);}
fn pbr_fetch(set_id:u32,map_id:u32,mip:u32,p:vec2<i32>)->vec4<f32>{let dim=max(1i,i32(PBR_MAX_DIMENSION)>>min(mip,PBR_MIP_LEVEL_COUNT-1u));let size=vec2<i32>(dim);let q=((p%size)+size)%size;var mip_offset=0u;for(var i=0u;i<mip;i++){let d=PBR_MAX_DIMENSION>>i;mip_offset+=d*d;}let idx=(set_id-1u)*PBR_MAPS_PER_SET*PBR_MIP_TEXEL_COUNT+map_id*PBR_MIP_TEXEL_COUNT+mip_offset+u32(q.y*dim+q.x);let word=pbr_texels[idx];var c=vec4<f32>(f32(word&255u),f32((word>>8u)&255u),f32((word>>16u)&255u),f32((word>>24u)&255u))/255.;if(map_id==0u){c=vec4<f32>(decode_srgb(c.x),decode_srgb(c.y),decode_srgb(c.z),c.w);}return c;}
fn pbr_level(set_id:u32,map_id:u32,uv:vec2<f32>,mip:u32)->vec4<f32>{let dim=max(1u,PBR_MAX_DIMENSION>>min(mip,PBR_MIP_LEVEL_COUNT-1u));let p=fract(uv)*f32(dim)-.5;let p0=vec2<i32>(floor(p));let t=fract(p);let a=mix(pbr_fetch(set_id,map_id,mip,p0),pbr_fetch(set_id,map_id,mip,p0+vec2<i32>(1,0)),t.x);let b=mix(pbr_fetch(set_id,map_id,mip,p0+vec2<i32>(0,1)),pbr_fetch(set_id,map_id,mip,p0+vec2<i32>(1,1)),t.x);return mix(a,b,t.y);}
fn pbr_sample(set_id:u32,map_id:u32,uv:vec2<f32>,lod:f32)->vec4<f32>{let l=clamp(lod,0.,8.);let lo=u32(floor(l));let hi=min(lo+1u,8u);return mix(pbr_level(set_id,map_id,uv,lo),pbr_level(set_id,map_id,uv,hi),fract(l));}
fn face_ao_corner(out:vec3<i32>,axis:u32,su:f32,sv:f32)->f32{let u_axis=select(vec3<f32>(1.,0.,0.),vec3<f32>(0.,1.,0.),axis==0u);let v_axis=select(vec3<f32>(0.,1.,0.),vec3<f32>(0.,0.,1.),axis!=2u);let a=out+vec3<i32>(u_axis*su);let b=out+vec3<i32>(v_axis*sv);let c=a+vec3<i32>(v_axis*sv);let sa=voxel(a)!=i32(u.frame.w);let sb=voxel(b)!=i32(u.frame.w);let sc=voxel(c)!=i32(u.frame.w);if(sa&&sb){return 0.;}return f32(3-i32(sa)-i32(sb)-i32(sc))/3.;}
fn face_ao(h:Hit)->f32{let axis=select(select(2u,1u,h.normal.y!=0),0u,h.normal.x!=0);var uv=h.point.xy;if(h.normal.x!=0){uv=h.point.yz;}else if(h.normal.y!=0){uv=vec2<f32>(h.point.x,h.point.z);}uv=fract(uv);let out=h.cell+h.normal;let cv=vec4<f32>(face_ao_corner(out,axis,-1.,-1.),face_ao_corner(out,axis,1.,-1.),face_ao_corner(out,axis,-1.,1.),face_ao_corner(out,axis,1.,1.));let top=mix(cv.x,cv.y,uv.x);let bot=mix(cv.z,cv.w,uv.x);return mix(top,bot,uv.y);}
fn cell_transform(cell:vec3<i32>,uv:vec2<f32>)->vec3<f32>{let q=u32((hash4(cell.x,cell.y,cell.z,97)+1.)*8.);var p=uv;if((q&4u)!=0u){p.x=1.-p.x;}if((q&8u)!=0u){p.y=1.-p.y;}for(var i=0u;i<(q&3u);i++){p=vec2<f32>(p.y,1.-p.x);}return vec3<f32>(p,.92+hash4(cell.x,cell.y,cell.z,103)*.08);}
fn albedo(h:Hit)->f32{let m=materials[h.id];if(m.info.x==0u){return m.params.x;}let axis=select(select(2u,1u,h.normal.y!=0),0u,h.normal.x!=0);var base=0.;var high=0.;var row=0;var rp=0.;var hor=0.;if(axis==2u){let f=fract(h.point.xy);let d=length(f-vec2<f32>(.5));base=select(.4,.3,m.info.x==3u);if(d<.27){base*=1.25;}else if(d<.34){base*=.6;}if(any(f<vec2<f32>(.025))||any(f>vec2<f32>(.975))){base*=.65;}base*=1.+.05*hash4(h.cell.x,h.cell.y,h.cell.z,7);}else if(m.info.x==2u){high=h.point[select(0u,1u,axis==0u)];rp=h.point.z*2.;row=i32(floor(rp));hor=high+select(0.,.5,(row&1)==1);if(fract(rp)<.10||fract(hor)<.06){base=.2;}else{base=.52+.10*hash4(i32(floor(hor)),row,i32(axis)+3*h.normal[axis],h.cell.x+h.cell.y*31);}}else{base=m.params.x;}if(u.style.z>0.5&&m.info.w>0u){var uv=fract(h.point.xy);if(h.normal.x!=0){uv=fract(h.point.yz);}else if(h.normal.y!=0){uv=fract(vec2<f32>(h.point.x,h.point.z));}var tex_id=m.info.w;var uv_var=uv;var tint=1.;if(m.params.w>.5){let tr=cell_transform(h.cell,uv);uv_var=tr.xy;tint=tr.z;if(tex_id==1u&&u32((hash4(h.cell.x,h.cell.y,h.cell.z,97)+1.)*8.)%4u==0u){tex_id=2u;}}let p=pbr_sample(tex_id,0u,uv_var*vec2<f32>(m.params.y,m.params.z),log2(max(h.t,1.)));let lum=dot(p.xyz,vec3<f32>(.2126,.7152,.0722));return base*clamp(.35+1.3*lum,.35,1.3)*tint;}if(u.style.y==0.||m.info.y==0u){return base;}var detail=1.;if(m.info.y==1u&&m.info.x==2u){let v=texel_fade(brick,vec2<f32>(hor,rp),h.t,7.,17.,.50);let variation=.92+.16*hash4(i32(floor(hor)),row,i32(axis)+h.normal[axis],i32(m.info.z));detail=clamp(v/.50*variation,.7,1.3);}else if(m.info.y==2u&&(m.info.x==1u||m.info.x==3u)){var uv=fract(h.point.xy);if(h.normal.x!=0){uv=fract(h.point.yz);}else if(h.normal.y!=0){uv=fract(vec2<f32>(h.point.x,h.point.z));}detail=clamp(texel_fade(noise_tex,uv,h.t,5.,14.,.46)/.46,.7,1.3);}return base*mix(1.,detail,clamp(u.style.y,0.,1.));}
fn finish(g:f32,p:vec2<i32>,uv:vec2<f32>)->vec4<f32>{let tp=clamp(g,0.,2.)*511.5;let ti=u32(tp);let mapped=mix(tone[ti],tone[min(ti+1u,1023u)],fract(tp));let vig=1.-dot(uv,uv)*(.45+.5*u.projection.y);let grain=hash4(p.x,p.y,i32(u.frame.x),5)*.025*(1.+3.*u.projection.y)*select(1.,0.,u.projection.w>1.5);let value=clamp(mapped*vig+grain,0.,1.)*1023.;let gi=u32(value);let level=mix(gamma[gi],gamma[min(gi+1u,1023u)],fract(value));let bayer=array<u32,16>(0u,8u,2u,10u,12u,4u,14u,6u,3u,11u,1u,9u,15u,7u,13u,5u);let b=(u32(p.y)&3u)*4u+(u32(p.x)&3u);let rgb=palette[u32(clamp(floor(level+(f32(bayer[b])+.5)/16.),0.,13.))];return vec4<f32>(f32((rgb>>16u)&255u)/255.,f32((rgb>>8u)&255u)/255.,f32(rgb&255u)/255.,1.);}
@fragment fn fs_world(@builtin(position) pos:vec4<f32>)->@location(0) vec4<f32>{
 let p=vec2<i32>(pos.xy);
 let uv=vec2<f32>((f32(p.x)+.5)/f32(u.dims.w)*2.-1.,1.-(f32(p.y)+.5)/f32(u.frame.z)*2.);
 let th=tan(u.projection.x*.5);
 let phase=f32(u.frame.x)*.016;let warp=clamp(u.projection.y,0.,1.)*.003*select(1.,0.,u.projection.w>1.5);
 let ray_uv=uv+vec2<f32>(sin(uv.y*48.+phase),sin(uv.x*32.-phase*.7))*warp;
 let d=normalize(u.forward.xyz+u.right.xyz*ray_uv.x*th+u.up.xyz*ray_uv.y*th*f32(u.frame.z)/f32(u.dims.w));
 let wh=cast_world(u.eye.xyz,d,u.projection.z);
 let eh=cast_entities(u.eye.xyz,d,select(1e30,wh.t,wh.ok));
 var sh=SpriteHit(false,1e30,0.,0.,0u,0u,0u);
 var sprite_color=vec4<f32>(0.);
 var sprite_normal=vec4<f32>(.5,.5,1.,0.);
 var sprite_emissive=vec4<f32>(0.);
 if(u.style.w>0.5){sh=cast_sprites(u.eye.xyz,d,select(1e30,wh.t,wh.ok));if(sh.ok){sprite_color=sprite_fetch(0u,sh);sprite_normal=sprite_fetch(1u,sh);sprite_emissive=sprite_fetch(2u,sh);}}
 let cone=clamp((dot(d,u.forward.xyz)-.86)/(.985-.86),0.,1.);
 let c=cone*cone*(3.-2.*cone);
 var g=0.;
 if(sh.ok&&sprite_color.a>.5&&(!eh.ok||sh.t<eh.t)){
  let n=normalize(sprite_normal.xyz*2.-vec3<f32>(1.));
  let ndotl=max(dot(n,-d),0.);
  let rgb=vec3<f32>(decode_srgb(sprite_color.r),decode_srgb(sprite_color.g),decode_srgb(sprite_color.b));
  let base=dot(rgb,vec3<f32>(.2126,.7152,.0722));
  g=base*light(sh.t,ndotl,c)+sprite_emissive.r*.85;
 }else if(eh.ok&&(!wh.ok||eh.t<wh.t)){
  g=.10*light(eh.t,eh.ndotl,c);
 }else if(wh.ok){
  let shadow_c=shadow_cone(wh,c);
  var contact_shadow=1.;
  if(wh.normal.z>0&&u.sprite.x>0u){for(var i=0u;i<min(u.frame.y,u.entity_limits.x);i++){let e=entities[i];if(e.render.x>0.5){let delta=(wh.point.xy-e.pos_yaw.xy)/max(e.shadow.xy,vec2<f32>(.001));let edge=clamp(1.-length(delta),0.,1.);contact_shadow*=1.-.38*edge*edge*(3.-2.*edge);}}}
  let axis=select(select(2u,1u,wh.normal.y!=0),0u,wh.normal.x!=0);
  let strength=clamp(u.style.x,0.,1.)*clamp(1.-wh.t/4.,0.,1.);
  let ao=mix(1.,face_ao(wh),strength);
  let m=materials[wh.id];
  if(u.style.z>0.5&&m.info.w>0u){
   var tuv=fract(wh.point.xy);if(wh.normal.x!=0){tuv=fract(wh.point.yz);}else if(wh.normal.y!=0){tuv=fract(vec2<f32>(wh.point.x,wh.point.z));}
   var tex_id=m.info.w;var tuv_var=tuv;var tint=1.;if(m.params.w>.5){let tr=cell_transform(wh.cell,tuv);tuv_var=tr.xy;tint=tr.z;if(tex_id==1u&&u32((hash4(wh.cell.x,wh.cell.y,wh.cell.z,97)+1.)*8.)%4u==0u){tex_id=2u;}}let tuv_scaled=tuv_var*vec2<f32>(m.params.y,m.params.z);let lod=log2(max(wh.t,1.));
   let tn=pbr_sample(tex_id,1u,tuv_scaled,lod).xyz*2.-vec3<f32>(1.);
   var tangent=vec3<f32>(1.,0.,0.);var bitangent=vec3<f32>(0.,1.,0.);
   if(axis==0u){tangent=vec3<f32>(0.,f32(wh.normal.x),0.);bitangent=vec3<f32>(0.,0.,1.);}else if(axis==1u){tangent=vec3<f32>(1.,0.,0.);bitangent=vec3<f32>(0.,0.,1.);}
   let surface_normal=normalize(tangent*tn.x+bitangent*tn.y+vec3<f32>(wh.normal)*tn.z);
   let ndotl=clamp(dot(surface_normal,-d),0.,1.);let rough=clamp(pbr_sample(tex_id,2u,tuv_scaled,lod).x,.04,1.);let mat_ao=clamp(pbr_sample(tex_id,3u,tuv_scaled,lod).x,0.,1.);
   let diffuse=light(wh.t,ndotl,shadow_c)*ao*mat_ao;let spec=u.lighting.x*shadow_c/(1.+.06*wh.t*wh.t)*(1.-rough)*.22*pow(ndotl,2.+32.*(1.-rough))*clamp(1.-wh.t/max(u.lighting.w,.001),0.,1.);
   g=albedo(wh)*(diffuse+spec);
  }else{g=albedo(wh)*light(wh.t,abs(d[axis]),shadow_c)*ao;}
  g*=contact_shadow;
 }
 return finish(g,p,uv);
}

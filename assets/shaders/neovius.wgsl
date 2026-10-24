// Neovius — shader.gallery (neovius)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: ember

const SG_C0: vec3f = vec3f(0.984000, 0.451000, 0.133000);
const SG_C1: vec3f = vec3f(0.957000, 0.247000, 0.369000);
const SG_C2: vec3f = vec3f(0.965000, 0.620000, 0.043000);
const SG_C3: vec3f = vec3f(0.561000, 0.149000, 0.305000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// neovius (Manifold) - a raymarched flight through a Neovius surface, a triply-
// periodic minimal surface of fat rounded pockets joined by narrow necks - softer
// and more bulbous than gyroid or diamond. The camera drifts through; walls are
// normal-shaded with topographic contour ribbing wrapping each pocket, hue
// turning with depth into fog. Sibling on the same engine. Comments ASCII only
// (the headless poster compiler is fussy - no apostrophes, no pow of a negative
// base).
//
// Uniforms: u.time, u.resolution, vec2f(0.0), 1.0, u_palette[4]



fn wheelW(s: f32, c: f32) -> f32 {
 let d =abs(s-c);
  return max(0.0,1.0-min(d,4.0-d)); }
fn wheelCol(k: f32, c0: vec3f, c1: vec3f, c2: vec3f, c3: vec3f) -> vec3f {

  var s = fract(k)*4.0;
  let a = wheelW(s,0.0);
  var b =wheelW(s,1.0);
  var cc =wheelW(s,2.0);
  var dd =wheelW(s,3.0);
  return (c0*a+c1*b+c2*cc+c3*dd)/max(a+b+cc+dd,0.001);
}

fn gmap(p: vec3f) -> f32 {

  let F = max(param(0u),0.6);
  let agg =mix(0.4,2.6,clamp(param(13u),0.0,1.0));      // axial cell stretch
  var q = p*F; q = vec3f(q.x, q.y, q.z * (agg));
  let tw = param(11u)*0.6;
  let ta = q.z*tw;
  var cz =cos(ta);
  var sz =sin(ta);
  let _swXY = mat2x2<f32>(cz,-sz,sz,cz)*q.xy; q = vec3f(_swXY.x, _swXY.y, q.z);                        // twist around flight axis
  q+=param(4u)*0.5*sin(q.yzx*0.7+u.time*0.15);
  var s = sin(q);
  var c =cos(q);
  // Neovius, scaled to roughly unit range so the shell math matches the siblings
  let neo = (3.0*(c.x+c.y+c.z)+4.0*c.x*c.y*c.z)/7.0;
  let sch =(c.x+c.y+c.z)/1.7;                         // Schwarz-P, similar scale
  let dia =s.x*s.y*s.z + s.x*c.y*c.z + c.x*s.y*c.z + c.x*c.y*s.z; // Schwarz-D
  var g = mix(neo,sch,clamp(param(8u),0.0,1.0));
  g=mix(g,dia,clamp(param(14u),0.0,1.0));                   // crystallize toward diamond
  let g2 =g-param(12u)*0.9;                               // iso-level: walls -> pockets
  let th =mix(0.02,1.3,clamp(param(1u),0.0,1.0));       // wider thickness range
  return (abs(g2)-th)/(F*max(1.0,agg)*(2.4+abs(tw)*2.0+param(14u)*0.6));
}

fn calcN(p: vec3f) -> vec3f {

  let e = vec2f(0.0015,0.0);
  return normalize(vec3f(
    gmap(p+e.xyy)-gmap(p-e.xyy),
    gmap(p+e.yxy)-gmap(p-e.yxy),
    gmap(p+e.yyx)-gmap(p-e.yyx)));
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let uv = (fc-0.5*res)/min(res.x,res.y);
  let t = u.time;

  let c0 = SG_C0;
  var c1 =SG_C1;
  var c2 =SG_C2;
  var c3 =SG_C3;
  // start off the x=z=0 symmetry axis so structure reads on load (see diamond)
  let ro = vec3f(0.9+sin(t*0.05)*0.5, 0.6+cos(t*0.06)*0.5, 0.7+t*param(2u));
  var rd = normalize(vec3f(uv,1.0));
  let a = param(7u)*0.0174532925-t*0.02;
  let ca = cos(a);
  var sa =sin(a);
  let _swXY = mat2x2<f32>(ca,-sa,sa,ca)*rd.xy; rd = vec3f(_swXY.x, _swXY.y, rd.z);

  var dist = 0.0;
  var hit =0.0;
  var steps =0.0;
  var p = ro;
  for (var i: i32 = 0; i<150; i += 1){
    p=ro+rd*dist;
    let d = gmap(p);
    if (d<0.0008){ hit=1.0; break; }
    dist+=d*0.5;
    steps+=1.0;
    if (dist>20.0) { break; }
  }
  steps=steps/150.0;

  let fogc = wheelCol(0.64+t*0.01,c0,c1,c2,c3)*0.05;
  var col = fogc;
  if (hit>0.5){
    let n = calcN(p);
    let lp = normalize(vec3f(0.6,0.6,-0.5));
    let diff = 0.34+0.66*max(dot(n,lp),0.0);
    let rim = pow(1.0-max(dot(n,-rd),0.0),2.2);
    let ao = 1.0-steps;
    let hue = param(5u)*(dist*0.08+0.2*n.y+0.12*n.z)+t*0.03;
    let base = wheelCol(hue,c0,c1,c2,c3);
    col=base*diff*(0.5+0.5*ao) + base*rim*(0.5+0.8*param(3u));

    let sil = 1.0-abs(dot(n,rd));
    let ew = 0.04+0.22*clamp(param(9u),0.0,1.0);
    let edgeLine = smoothstep(1.0-ew,1.0-ew*0.4,sil);
    col+=base*edgeLine*clamp(param(9u),0.0,1.0)*(0.7+0.7*param(3u));

    let cf = fract(length(p.xy)*mix(0.0,2.4,clamp(param(10u),0.0,1.0))-t*0.05);
    let cont = smoothstep(0.5,0.42,abs(cf-0.5))*clamp(param(10u),0.0,1.0);
    col+=base*cont*(0.5+0.6*param(3u));
  }

  let fog = 1.0-exp(-dist*mix(0.05,0.30,clamp(param(6u),0.0,1.0)));
  col=mix(col,fogc,clamp(fog,0.0,1.0));
  col*=mix(1.0,0.7,smoothstep(0.6,1.1,length(uv)));

  return vec4f(col,1.0);
}

// Gyroid — shader.gallery (gyroid)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: prism

const SG_C0: vec3f = vec3f(0.376000, 0.647000, 0.949000);
const SG_C1: vec3f = vec3f(0.345000, 0.847000, 0.612000);
const SG_C2: vec3f = vec3f(0.965000, 0.792000, 0.318000);
const SG_C3: vec3f = vec3f(0.949000, 0.408000, 0.471000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// gyroid (Manifold) - a raymarched flight through a gyroid, a triply-periodic
// minimal surface that tiles all of space into one smooth interwoven labyrinth.
// The camera drifts forward through the lattice; the thickened surface glows
// volumetrically, shaded by its own normals, hue turning with depth, fading into
// fog. A real 3D engine, not strokes on black - it fills the frame because it is
// a solid the eye flies through. Comments ASCII only (the headless poster
// compiler is fussy - no apostrophes, no pow of a negative base).
//
// Uniforms: u.time, u.resolution, vec2f(0.0), 1.0, u_palette[4]



fn wheelW(s: f32, c: f32) -> f32 {
 let d =abs(s-c);
  return max(0.0,1.0-min(d,4.0-d)); }
fn wheelCol(k: f32, c0: vec3f, c1: vec3f, c2: vec3f, c3: vec3f) -> vec3f {

  let s = fract(k)*4.0;
  let a = wheelW(s,0.0);
  var b =wheelW(s,1.0);
  var cc =wheelW(s,2.0);
  var dd =wheelW(s,3.0);
  return (c0*a+c1*b+c2*cc+c3*dd)/max(a+b+cc+dd,0.001);
}

fn gmap(p: vec3f) -> f32 {

  let F = max(param(0u),0.6);
  var q = p*F;
  let tw = param(11u)*0.6;
  let az = q.z*tw;
  var cz =cos(az);
  var sz =sin(az);
  let _swXY = mat2x2<f32>(cz,-sz,sz,cz)*q.xy; q = vec3f(_swXY.x, _swXY.y, q.z);                   // twist around flight axis
  q+=param(4u)*0.5*sin(q.yzx*0.7+u.time*0.15);      // slow domain warp
  let gyr =dot(sin(q),cos(q.yzx));              // gyroid implicit surface
  let sch =cos(q.x)+cos(q.y)+cos(q.z);          // Schwarz-P chamber lattice
  var g =mix(gyr,sch,clamp(param(8u),0.0,1.0)); // morph between the two shapes
  g=g-param(12u)*1.1;                                 // iso-level: walls -> pockets
  // thin walls keep the camera in the open void so rays travel and strike the
  // lattice at varying depths (fat walls put the camera inside the solid -> every
  // ray hits at zero distance -> a flat uniform wash).
  let th = mix(0.05,0.9,clamp(param(1u),0.0,1.0));
  // divide by ~Lipschitz bound (F * ~2.6) so the step underestimates distance
  // and the march actually lands on the walls instead of overshooting them.
  return (abs(g)-th)/(F*(2.6+abs(tw)*2.0));
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
  // camera flies forward, with a gentle drift so the labyrinth never repeats flat
  // start off the x=z=0 symmetry axis so structure reads on load (see diamond)
  let ro = vec3f(0.9+sin(t*0.07)*0.6, 0.6+cos(t*0.05)*0.6, 0.7+t*param(2u));
  var rd = normalize(vec3f(uv,1.0));
  let a = param(7u)*0.0174532925+t*0.03;
  let ca = cos(a);
  var sa =sin(a);
  let _swXY = mat2x2<f32>(ca,-sa,sa,ca)*rd.xy; rd = vec3f(_swXY.x, _swXY.y, rd.z);                // roll the view

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

  let fogc = wheelCol(0.62+t*0.01,c0,c1,c2,c3)*0.05;
  var col =fogc;                                // escaped rays = deep background
  if (hit>0.5){
    let n = calcN(p);
    let lp = normalize(vec3f(0.5,0.7,-0.6));
    let diff = 0.32+0.68*max(dot(n,lp),0.0);
    let rim =pow(1.0-max(dot(n,-rd),0.0),2.5); // bright facing-away edges
    let ao =1.0-steps;                         // crude crevice darkening
    let hue = param(5u)*(dist*0.08+0.2*n.y+0.1*n.x)+t*0.03;
    let base = wheelCol(hue,c0,c1,c2,c3);
    col=base*diff*(0.5+0.5*ao) + base*rim*(0.5+0.8*param(3u));

    // crisp silhouette line: a bright contour where the wall turns away from view
    let sil = 1.0-abs(dot(n,rd));
    let ew = 0.04+0.22*clamp(param(9u),0.0,1.0);
    let edgeLine = smoothstep(1.0-ew,1.0-ew*0.4,sil);
    col+=base*edgeLine*clamp(param(9u),0.0,1.0)*(0.7+0.7*param(3u));

    // topographic lines wrapping the surface by world height
    let cf = fract(p.y*mix(0.0,3.0,clamp(param(10u),0.0,1.0))-t*0.05);
    let cont = smoothstep(0.5,0.42,abs(cf-0.5))*clamp(param(10u),0.0,1.0);
    col+=base*cont*(0.5+0.6*param(3u));
  }

  // depth fog so the far lattice recedes into the tinted ground
  let fog = 1.0-exp(-dist*mix(0.05,0.30,clamp(param(6u),0.0,1.0)));
  col=mix(col,fogc,clamp(fog,0.0,1.0));

  // mild vignette
  let vr = length(uv);
  col*=mix(1.0,0.7,smoothstep(0.6,1.1,vr));

  return vec4f(col,1.0);
}

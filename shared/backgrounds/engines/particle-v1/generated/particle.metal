// language: metal2.0
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint size1;
    uint size2;
    uint size3;
    uint size4;
    uint size5;
    uint size8;
    uint size9;
};

struct Uniforms {
    float time;
    float aspect;
    float pointSize;
    float scale;
    metal::float3 rotation;
    metal::float3 position;
    metal::float4 appearance;
    metal::float4 model;
    metal::float4 screenFit;
    metal::float4 graph;
    metal::float2 viewport;
    metal::float2 pad;
};
typedef metal::float4 type_4[1];
struct ParticleSample {
    metal::float2 clip;
    float pointSizePx;
    char _pad2[4];
    metal::float4 color;
};
struct VertexOut {
    metal::float4 position;
    metal::float2 uv;
    char _pad2[8];
    metal::float4 color;
};
struct type_8 {
    metal::float2 inner[6];
};
struct LineOut {
    metal::float4 position;
    metal::float4 color;
};
constant uint GRAPH_NODE_COUNT = 768u;
constant uint GRAPH_MAX_CONNECTIONS = 3u;

metal::float3 hsvToRGB(
    metal::float3 hsv
) {
    metal::float4 constants = metal::float4(1.0, 0.6666667, 0.33333334, 3.0);
    metal::float3 p_3 = metal::abs((metal::fract(metal::float3(hsv.x) + constants.xyz) * 6.0) - constants.www);
    return hsv.z * metal::mix(metal::float3(constants.x), metal::clamp(p_3 - metal::float3(constants.x), metal::float3(0.0), metal::float3(1.0)), hsv.y);
}

float flowHash(
    metal::float3 p0_
) {
    metal::float3 p = {};
    p = metal::fract(p0_ * 0.1031);
    metal::float3 _e5 = p;
    metal::float3 _e6 = p;
    metal::float3 _e13 = p;
    p = _e13 + metal::float3(metal::dot(_e5, _e6.yzx + metal::float3(33.33)));
    float _e16 = p.x;
    float _e18 = p.y;
    float _e21 = p.z;
    return metal::fract((_e16 + _e18) * _e21);
}

float flowNoise(
    metal::float3 p_1
) {
    metal::float3 fraction = {};
    metal::float3 cell = metal::floor(p_1);
    fraction = metal::fract(p_1);
    metal::float3 _e4 = fraction;
    metal::float3 _e5 = fraction;
    metal::float3 _e7 = fraction;
    fraction = (_e4 * _e5) * (metal::float3(3.0) - (2.0 * _e7));
    float _e19 = flowHash(cell + metal::float3(0.0, 0.0, 0.0));
    float _e25 = flowHash(cell + metal::float3(1.0, 0.0, 0.0));
    float _e27 = fraction.x;
    float x00_ = metal::mix(_e19, _e25, _e27);
    float _e34 = flowHash(cell + metal::float3(0.0, 1.0, 0.0));
    float _e40 = flowHash(cell + metal::float3(1.0, 1.0, 0.0));
    float _e42 = fraction.x;
    float x10_ = metal::mix(_e34, _e40, _e42);
    float _e49 = flowHash(cell + metal::float3(0.0, 0.0, 1.0));
    float _e55 = flowHash(cell + metal::float3(1.0, 0.0, 1.0));
    float _e57 = fraction.x;
    float x01_ = metal::mix(_e49, _e55, _e57);
    float _e64 = flowHash(cell + metal::float3(0.0, 1.0, 1.0));
    float _e70 = flowHash(cell + metal::float3(1.0, 1.0, 1.0));
    float _e72 = fraction.x;
    float x11_ = metal::mix(_e64, _e70, _e72);
    float _e75 = fraction.y;
    float _e78 = fraction.y;
    float _e81 = fraction.z;
    return metal::mix(metal::mix(x00_, x10_, _e75), metal::mix(x01_, x11_, _e78), _e81);
}

metal::float2 sampleParametricWaves(
    uint id,
    float t
) {
    float i = 9999.0 - static_cast<float>(id);
    float y = i / 235.0;
    float k = (4.0 + metal::cos((i / 9.0) - (t * 2.0))) * metal::cos(i / 35.0);
    float e = (y / 7.0) - 13.0;
    float d_1 = (metal::length(metal::float2(k, e)) + metal::sin((e / 9.0) + (t / 2.0))) - 4.0;
    float q = (2.0 * metal::sin(k * 3.0)) - (((y / 35.0) * k) * (9.0 + (k * metal::sin(((metal::cos(e) * 9.0) - (d_1 * 2.0)) + t))));
    float c = d_1 - t;
    return metal::float2((q + (40.0 * metal::cos(c))) + 200.0, (q * metal::sin(c)) + (d_1 * 35.0));
}

metal::float2 sampleTwinVortex(
    uint id_1,
    float t_1
) {
    float i_1 = 29999.0 - static_cast<float>(id_1);
    float m = metal::fmod(i_1, 2.0) * 3.0;
    float k_1 = 14.0 * metal::cos(i_1 / 39.0);
    float e_1 = (i_1 / 1200.0) - 13.0;
    float d_2 = (metal::dot(metal::float2(k_1, e_1), metal::float2(k_1, e_1)) / 59.0) + 1.0;
    float q_1 = (89.0 - (metal::sin(k_1) * d_2)) + (k_1 * ((8.0 / d_2) + metal::sin(((d_2 * 3.0) + (e_1 / 9.0)) - t_1)));
    float c_1 = (((d_2 * 0.45) - (metal::sin(t_1 - d_2) / 8.0)) - (t_1 / 8.0)) + m;
    return metal::float2((q_1 * metal::sin(c_1)) + 200.0, (((q_1 + 40.0) + (30.0 * metal::sin((c_1 * 2.0) + m))) * metal::cos(c_1)) + 200.0);
}

metal::float2 sampleOrbitalBloom(
    uint id_2,
    float t_2
) {
    float i_2 = 29999.0 - static_cast<float>(id_2);
    float y_1 = i_2 / 799.0;
    float k_2 = 5.0 * metal::cos(i_2 / 48.0);
    float e_2 = 5.0 * metal::cos(y_1 / 9.0);
    float divisor = 6.0 + metal::fmod(i_2, 4.0);
    float d_3 = metal::pow(metal::length(metal::float2(k_2, e_2)) / divisor, 4.0) + 4.0;
    float parityOffset = 80.0 * (1.0 + metal::fmod(i_2, 2.0));
    float q_2 = ((k_2 * (3.0 + ((e_2 / 2.0) * metal::sin(((d_3 * 8.0) + (k_2 / 9.0)) - t_2)))) - (3.0 * metal::sin((k_2 * d_3) / 3.0))) + parityOffset;
    float c_2 = (d_3 - (t_2 / 9.0)) + metal::fmod(i_2, 5.0);
    return metal::float2((q_2 * metal::sin(c_2)) + 200.0, (q_2 * metal::cos(((c_2 - metal::fmod(i_2, 2.0)) + (metal::fmod(i_2, 5.0) * 3.0)) + 7.0)) + 200.0);
}

uint naga_div(uint lhs, uint rhs) {
    return lhs / metal::select(rhs, 1u, rhs == 0u);
}

uint naga_mod(uint lhs, uint rhs) {
    return lhs % metal::select(rhs, 1u, rhs == 0u);
}

metal::float2 sampleHexagonalRosette(
    uint id_3,
    float t_3
) {
    metal::float2 centered = {};
    uint sidx = naga_div(id_3, 6u);
    uint rot = naga_mod(id_3, 6u);
    float i_3 = 19999.0 - static_cast<float>(sidx);
    float k_3 = metal::fmod(i_3, 25.0) - 12.0;
    float e_3 = i_3 / 800.0;
    float d_4 = 7.0 * metal::cos((metal::length(metal::float2(k_3, e_3)) / 3.0) + (t_3 / 2.0));
    centered = metal::float2((k_3 * 4.0) + ((d_4 * k_3) * metal::sin((d_4 + (e_3 / 9.0)) + t_3)), ((e_3 * 2.0) - (d_4 * 9.0)) - ((d_4 * 9.0) * metal::cos(d_4 + t_3)));
    float angle = static_cast<float>(rot) * 1.0471976;
    float ca = metal::cos(angle);
    float sa = metal::sin(angle);
    float _e54 = centered.x;
    float _e57 = centered.y;
    float _e61 = centered.x;
    float _e64 = centered.y;
    centered = metal::float2((_e54 * ca) - (_e57 * sa), (_e61 * sa) + (_e64 * ca));
    metal::float2 _e68 = centered;
    return _e68 + metal::float2(200.0);
}

metal::float2 sampleTorusOrbit(
    uint id_4,
    float t_4,
    thread float& pointScale
) {
    metal::float3 world = {};
    metal::float3 local = {};
    if (id_4 < 512u) {
        float sidx_1 = static_cast<float>(id_4) + 0.5;
        float sphereY = 1.0 - ((2.0 * sidx_1) / 512.0);
        float sphereRadius = metal::sqrt(metal::max(0.0, 1.0 - (sphereY * sphereY)));
        float sphereAngle = sidx_1 * 2.3999631;
        metal::float3 unitSphere = metal::float3(metal::cos(sphereAngle) * sphereRadius, sphereY, metal::sin(sphereAngle) * sphereRadius);
        world = metal::float3((60.0 * metal::sin(t_4)) + (99.0 * metal::sin(-(t_4))), -20.0, 170.0 + (99.0 * metal::cos(t_4))) + (unitSphere * 4.0);
        pointScale = 0.75;
    } else {
        uint localID_1 = id_4 - 512u;
        uint torusID = naga_div(localID_1, 576u);
        uint torusPoint = naga_mod(localID_1, 576u);
        uint majorID = naga_div(torusPoint, 6u);
        uint tubeID = naga_mod(torusPoint, 6u);
        float major = (static_cast<float>(majorID) * 6.2831855) / 96.0;
        float tube = (static_cast<float>(tubeID) * 6.2831855) / 6.0;
        float radius = 80.0 + (4.0 * metal::cos(tube));
        local = metal::float3(radius * metal::cos(major), radius * metal::sin(major), 4.0 * metal::sin(tube));
        float _e84 = local.x;
        float _e86 = local.y;
        float _e89 = local.z;
        float _e93 = local.y;
        float _e96 = local.z;
        local = metal::float3(_e84, (_e86 * 0.6967067) - (_e89 * 0.7173561), (_e93 * 0.7173561) + (_e96 * 0.6967067));
        float _e102 = local.x;
        local.x = _e102 + 120.0;
        float ringAngle = ((static_cast<float>(torusID) * 3.1415927) / 32.0) + t_4;
        float cy = metal::cos(ringAngle);
        float sy = metal::sin(ringAngle);
        float _e113 = local.x;
        float _e116 = local.z;
        float _e120 = local.y;
        float _e122 = local.x;
        float _e126 = local.z;
        world = metal::float3((_e113 * cy) + (_e116 * sy), _e120, (-(_e122) * sy) + (_e126 * cy));
        metal::float3 _e136 = world;
        world = _e136 + metal::float3(60.0 * metal::sin(t_4), -20.0, 170.0);
        pointScale = 0.42;
    }
    float _e141 = world.z;
    float perspective = 346.41016 / metal::max(72.0, 346.41016 - _e141);
    float _e147 = world.x;
    float _e152 = world.y;
    return metal::float2(200.0 + (_e147 * perspective), 200.0 + (_e152 * perspective));
}

metal::float2 sampleSphereTorus(
    uint id_5,
    float t_5,
    thread float& pointScale_1
) {
    metal::float3 local_1 = {};
    uint y_2 = naga_div(id_5, 80u);
    uint x = naga_mod(id_5, 80u);
    float v = ((static_cast<float>(y_2) + t_5) * 0.07853982) * 2.0;
    float u_2 = (static_cast<float>(x) + t_5) * 0.07853982;
    float ring = 2.0 + metal::sin(v);
    local_1 = metal::float3((ring * metal::cos(u_2)) * 90.0, (ring * metal::sin(u_2)) * 90.0, metal::cos(v) * 90.0);
    float _e33 = local_1.x;
    float _e36 = local_1.z;
    float _e40 = local_1.y;
    float _e42 = local_1.x;
    float _e46 = local_1.z;
    local_1 = metal::float3((_e33 * 0.87758255) + (_e36 * -0.47942555), _e40, (-(_e42) * -0.47942555) + (_e46 * 0.87758255));
    float _e53 = local_1.x;
    float _e55 = local_1.y;
    float _e58 = local_1.z;
    float _e62 = local_1.y;
    float _e65 = local_1.z;
    local_1 = metal::float3(_e53, (_e55 * 0.87758255) - (_e58 * 0.47942555), (_e62 * 0.47942555) + (_e65 * 0.87758255));
    float _e71 = local_1.z;
    float perspective_1 = 519.61523 / metal::max(72.0, 519.61523 - _e71);
    pointScale_1 = metal::max(0.1, metal::cos(v) + 0.3);
    float _e82 = local_1.x;
    float _e87 = local_1.y;
    return metal::float2(300.0 + (_e82 * perspective_1), 300.0 + (_e87 * perspective_1));
}

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

ParticleSample particleSample(
    uint id_6,
    Uniforms u,
    device type_4 const& flowParticles,
    device type_4 const& flowHistory,
    constant _mslBufferSizes& _buffer_sizes
) {
    metal::float2 pixel = {};
    float canvasSize = 400.0;
    float pointScale_2 = 1.0;
    float trailAlpha = 1.0;
    metal::float3 renderColor = {};
    uint localID = {};
    float d = 330.0;
    uint ringPointCount = 0u;
    metal::float2 p_2 = {};
    ParticleSample out_2 = {};
    renderColor = u.appearance.xyz;
    float style = u.model.x;
    if (style < 0.5) {
        float t_6 = u.time * 1.1780972;
        metal::float2 _e19 = sampleParametricWaves(id_6, t_6);
        pixel = _e19;
    } else {
        if (style < 1.5) {
            float t_7 = u.time * 2.0943952;
            metal::float2 _e25 = sampleTwinVortex(id_6, t_7);
            pixel = _e25;
        } else {
            if (style < 2.5) {
                float t_8 = u.time * 1.5707964;
                metal::float2 _e31 = sampleOrbitalBloom(id_6, t_8);
                pixel = _e31;
            } else {
                if (style < 3.5) {
                    float t_9 = u.time * 0.3926991;
                    metal::float2 _e37 = sampleHexagonalRosette(id_6, t_9);
                    pixel = _e37;
                } else {
                    if (style < 4.5) {
                        uint particleID = naga_div(id_6, 12u);
                        uint trailID = naga_mod(id_6, 12u);
                        uint latestSlot = naga_f2u32(u.model.y + 0.5);
                        uint slot_2 = naga_mod((latestSlot + 12u) - trailID, 12u);
                        metal::float4 s = flowHistory[(particleID * 12u) + slot_2];
                        pixel = s.xy;
                        canvasSize = 720.0;
                        pointScale_2 = metal::max(0.16, s.z / 3.0);
                        trailAlpha = s.w * metal::exp(-(static_cast<float>(trailID)) * 0.2);
                    } else {
                        if (style < 5.5) {
                            float prime = flowParticles[id_6].x;
                            float t_10 = 1.0 + (u.time * 0.000003);
                            pixel = metal::float2(((prime * metal::sin(prime * t_10)) / 99.0) + 400.0, ((prime * metal::cos(prime * t_10)) / 99.0) + 400.0);
                            canvasSize = 800.0;
                            pointScale_2 = 0.38;
                        } else {
                            if (style < 6.5) {
                                metal::float2 _e103 = sampleTorusOrbit(id_6, u.time * 0.3, pointScale_2);
                                pixel = _e103;
                            } else {
                                if (style < 7.5) {
                                    uint baseID = naga_div(id_6, 8u);
                                    uint trailID_1 = naga_mod(id_6, 8u);
                                    localID = baseID;
                                    uint2 loop_bound = uint2(4294967295u);
                                    while(true) {
                                        if (metal::all(loop_bound == uint2(0u))) { break; }
                                        loop_bound -= uint2(loop_bound.y == 0u, 1u);
                                        float _e114 = d;
                                        ringPointCount = naga_f2u32(metal::ceil(3.1415927 * _e114));
                                        uint _e119 = localID;
                                        uint _e120 = ringPointCount;
                                        if (_e119 < _e120) {
                                            break;
                                        }
                                        uint _e122 = ringPointCount;
                                        uint _e123 = localID;
                                        localID = _e123 - _e122;
                                        float _e126 = d;
                                        d = _e126 - 30.0;
                                    }
                                    float t_11 = metal::max(0.0, (u.time * 30.0) - (static_cast<float>(trailID_1) * 0.85));
                                    uint _e137 = localID;
                                    float _e141 = d;
                                    float r = (static_cast<float>(_e137) * 2.0) / _e141;
                                    float _e143 = d;
                                    float tangent = metal::tan((_e143 / 199.0) - (t_11 / 99.0));
                                    float energy = metal::min(tangent * tangent, 64.0);
                                    float _e155 = d;
                                    float _e158 = flowNoise(metal::float3(r * 99.0, _e155, 0.0));
                                    float _e159 = d;
                                    float _e164 = d;
                                    float _e170 = d;
                                    float radius_1 = _e159 + ((((metal::sin((r * 9.0) + (((t_11 / 9.0) * _e164) / 720.0)) * _e170) * 0.25) * _e158) * energy);
                                    float angle_1 = r - 1.5707964;
                                    pixel = metal::float2((metal::cos(angle_1) * radius_1) + 360.0, (metal::sin(angle_1) * radius_1) + 360.0);
                                    canvasSize = 720.0;
                                    pointScale_2 = 0.9;
                                    float _e190 = d;
                                    float hue = metal::clamp(_e190, 0.0, 255.0) / 255.0;
                                    metal::float3 _e199 = hsvToRGB(metal::float3(hue, 0.19607843, 1.0));
                                    renderColor = _e199 * u.appearance.xyz;
                                    float sourceAlpha = metal::clamp(0.7 / metal::max(energy, 0.025), 0.018, 0.82);
                                    trailAlpha = sourceAlpha * metal::exp(-(static_cast<float>(trailID_1)) * 0.32);
                                } else {
                                    float t_12 = metal::fmod(u.time * 1.2, 1.0);
                                    metal::float2 _e221 = sampleSphereTorus(id_6, t_12, pointScale_2);
                                    pixel = _e221;
                                    canvasSize = 600.0;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    float _e223 = canvasSize;
    float center = _e223 * 0.5;
    float _e227 = pixel.x;
    float _e231 = pixel.y;
    p_2 = metal::float2((_e227 - center) / center, (center - _e231) / center);
    float cz = metal::cos(u.rotation.z);
    float sz = metal::sin(u.rotation.z);
    float _e243 = p_2.x;
    float _e246 = p_2.y;
    float _e250 = p_2.x;
    float _e253 = p_2.y;
    p_2 = metal::float2((_e243 * cz) - (_e246 * sz), (_e250 * sz) + (_e253 * cz));
    metal::float2 _e264 = p_2;
    p_2 = _e264 * (u.scale * metal::exp(u.position.z * 0.08));
    metal::float2 _e273 = p_2;
    p_2 = _e273 * metal::float2(metal::cos(u.rotation.y), metal::cos(u.rotation.x));
    metal::float2 _e279 = p_2;
    p_2 = _e279 + (u.position.xy * 0.25);
    metal::float2 _e286 = p_2;
    p_2 = _e286 * metal::max(metal::float2(0.05), u.screenFit.yz);
    if (u.screenFit.x < 0.5) {
        float _e296 = p_2.x;
        p_2.x = _e296 / metal::max(0.1, u.aspect);
    }
    metal::float2 _e300 = p_2;
    out_2.clip = _e300;
    float _e303 = pointScale_2;
    out_2.pointSizePx = metal::max(1.0, u.pointSize * _e303);
    float brightness = metal::max(0.05, u.appearance.w);
    metal::float3 _e312 = renderColor;
    float _e319 = trailAlpha;
    out_2.color = metal::float4(_e312 * brightness, metal::clamp(0.38 * brightness, 0.08, 1.0) * _e319);
    ParticleSample _e322 = out_2;
    return _e322;
}

struct vsMainInput {
};
struct vsMainOutput {
    metal::float4 position [[position]];
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float4 color [[user(loc1), center_perspective]];
};
vertex vsMainOutput vsMain(
  uint vi [[vertex_id]]
, constant Uniforms& u_1 [[buffer(0)]]
, device type_4 const& flowParticles [[buffer(1)]]
, device type_4 const& flowHistory [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(6)]]
) {
    type_8 offsets = type_8 {metal::float2(0.0, 0.0), metal::float2(1.0, 0.0), metal::float2(0.0, 1.0), metal::float2(0.0, 1.0), metal::float2(1.0, 0.0), metal::float2(1.0, 1.0)};
    VertexOut out = {};
    uint corner = naga_mod(vi, 6u);
    uint particleID_1 = naga_div(vi, 6u);
    Uniforms _e6 = u_1;
    ParticleSample _e7 = particleSample(particleID_1, _e6, flowParticles, flowHistory, _buffer_sizes);
    metal::float2 _e29 = offsets.inner[corner];
    metal::float2 f = static_cast<metal::float2>(_e29);
    metal::float2 _e40 = u_1.viewport;
    metal::float2 ndc = (((f - metal::float2(0.5)) * 2.0) * _e7.pointSizePx) / _e40;
    out.position = metal::float4(_e7.clip + ndc, 0.0, 1.0);
    out.uv = f;
    out.color = _e7.color;
    VertexOut _e52 = out;
    const auto _tmp = _e52;
    return vsMainOutput { _tmp.position, _tmp.uv, _tmp.color };
}


struct fsMainInput {
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float4 color [[user(loc1), center_perspective]];
};
struct fsMainOutput {
    metal::float4 member_1 [[color(0)]];
};
fragment fsMainOutput fsMain(
  fsMainInput varyings_1 [[stage_in]]
, metal::float4 position [[position]]
) {
    const VertexOut in = { position, varyings_1.uv, {}, varyings_1.color };
    metal::float2 centered_1 = (in.uv * 2.0) - metal::float2(1.0);
    float alpha = metal::smoothstep(1.0, 0.15, metal::length(centered_1)) * in.color.w;
    return fsMainOutput { metal::float4(in.color.xyz, alpha) };
}


struct flowUpdateInput {
};
kernel void flowUpdate(
  metal::uint3 gid [[thread_position_in_grid]]
, device type_4& flowParticlesMut [[buffer(1)]]
, device type_4& flowHistoryMut [[buffer(2)]]
, constant metal::uint4& flow [[buffer(4)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(6)]]
) {
    metal::float4 state = {};
    uint slot = 0u;
    uint id_7 = gid.x;
    uint particleCount = flow.z;
    uint historyCount = flow.w;
    if (id_7 >= particleCount) {
        return;
    }
    uint frame = flow.x;
    uint first = naga_mod(frame * 9u, particleCount);
    uint insertionOffset = naga_mod((id_7 + particleCount) - first, particleCount);
    bool spawned = insertionOffset < 9u;
    metal::float4 _e22 = flowParticlesMut[id_7];
    state = _e22;
    if (spawned) {
        uint newTick = ((frame * 9u) + insertionOffset) + 1u;
        state = metal::float4(metal::fmod(static_cast<float>(newTick) * 99.0, 720.0), 0.0, 0.0, 3.0);
        uint2 loop_bound_1 = uint2(4294967295u);
        bool loop_init = true;
        while(true) {
            if (metal::all(loop_bound_1 == uint2(0u))) { break; }
            loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
            if (!loop_init) {
                uint _e50 = slot;
                slot = _e50 + 1u;
            }
            loop_init = false;
            uint _e40 = slot;
            if (_e40 < historyCount) {
            } else {
                break;
            }
            {
                uint _e44 = slot;
                flowHistoryMut[(id_7 * historyCount) + _e44] = metal::float4(0.0);
            }
        }
    } else {
        float _e53 = state.w;
        if (_e53 <= 0.0) {
            return;
        }
    }
    float _e58 = state.w;
    state.w = _e58 * 0.997;
    float globalTick = static_cast<float>((frame + 1u) * 9u);
    float _e66 = state.x;
    float _e70 = state.y;
    float _e76 = flowNoise(metal::float3(_e66 / 720.0, _e70 / 9.0, globalTick / 720.0));
    if (_e76 > 0.4) {
        float _e81 = state.z;
        state.z = _e81 + 0.5;
        float _e85 = state.z;
        float _e86 = state.y;
        state.y = _e86 + _e85;
    } else {
        float _e96 = state.x;
        state.x = _e96 + ((metal::fmod(_e76, 0.1) > 0.05) ? 1.0 : -1.0);
        state.z = 0.0;
        float _e102 = state.y;
        state.y = _e102 + 0.5;
    }
    metal::float4 _e106 = state;
    flowParticlesMut[id_7] = _e106;
    uint historySlot = naga_mod(frame, historyCount);
    metal::float4 _e112 = state;
    float _e115 = state.w;
    flowHistoryMut[(id_7 * historyCount) + historySlot] = metal::float4(_e112.xy, _e115, 1.0);
    return;
}


struct graphPositionUpdateInput {
};
kernel void graphPositionUpdate(
  metal::uint3 gid_1 [[thread_position_in_grid]]
, constant Uniforms& u_1 [[buffer(0)]]
, device type_4 const& flowParticles [[buffer(1)]]
, device type_4 const& flowHistory [[buffer(2)]]
, device type_4& graphPositions [[buffer(5)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(6)]]
) {
    uint id_8 = gid_1.x;
    if (id_8 >= GRAPH_NODE_COUNT) {
        return;
    }
    float _e8 = u_1.model.w;
    uint sourceCount = metal::max(1u, naga_f2u32(_e8 + 0.5));
    uint sourceID = metal::min(sourceCount - 1u, naga_f2u32((static_cast<float>(id_8) * static_cast<float>(sourceCount)) / 768.0));
    Uniforms _e23 = u_1;
    ParticleSample _e24 = particleSample(sourceID, _e23, flowParticles, flowHistory, _buffer_sizes);
    bool visible = metal::all(metal::abs(_e24.clip) <= metal::float2(1.15)) && (_e24.color.w > 0.005);
    graphPositions[id_8] = metal::float4(_e24.clip, visible ? _e24.color.w : 0.0, static_cast<float>(sourceID));
    return;
}


struct graphConnectionUpdateInput {
};
kernel void graphConnectionUpdate(
  metal::uint3 gid_2 [[thread_position_in_grid]]
, constant Uniforms& gu [[buffer(0)]]
, device type_4 const& graphPositions [[buffer(5)]]
, device type_4& graphEdgesMut [[buffer(3)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(6)]]
) {
    float bestDistance0_ = {};
    float bestDistance1_ = {};
    float bestDistance2_ = {};
    uint bestID0_ = 4294967295u;
    uint bestID1_ = 4294967295u;
    uint bestID2_ = 4294967295u;
    uint candidate = {};
    uint slot_1 = 0u;
    uint id_9 = gid_2.x;
    if (id_9 >= GRAPH_NODE_COUNT) {
        return;
    }
    metal::float4 source = graphPositions[id_9];
    float _e10 = gu.graph.y;
    float threshold = metal::max(0.001, _e10);
    float thresholdSquared = threshold * threshold;
    bestDistance0_ = thresholdSquared;
    bestDistance1_ = thresholdSquared;
    bestDistance2_ = thresholdSquared;
    if (source.z > 0.0) {
        candidate = id_9 + 1u;
        uint2 loop_bound_2 = uint2(4294967295u);
        bool loop_init_1 = true;
        while(true) {
            if (metal::all(loop_bound_2 == uint2(0u))) { break; }
            loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
            if (!loop_init_1) {
                uint _e59 = candidate;
                candidate = _e59 + 1u;
            }
            loop_init_1 = false;
            uint _e29 = candidate;
            if (_e29 < GRAPH_NODE_COUNT) {
            } else {
                break;
            }
            {
                uint _e33 = candidate;
                metal::float4 tgt = graphPositions[_e33];
                if (tgt.z <= 0.0) {
                    continue;
                }
                metal::float2 delta = source.xy - tgt.xy;
                float distanceSquared = metal::dot(delta, delta);
                float _e43 = bestDistance2_;
                if (distanceSquared >= _e43) {
                    continue;
                }
                float _e45 = bestDistance0_;
                if (distanceSquared < _e45) {
                    float _e47 = bestDistance1_;
                    bestDistance2_ = _e47;
                    uint _e48 = bestID1_;
                    bestID2_ = _e48;
                    float _e49 = bestDistance0_;
                    bestDistance1_ = _e49;
                    uint _e50 = bestID0_;
                    bestID1_ = _e50;
                    bestDistance0_ = distanceSquared;
                    uint _e51 = candidate;
                    bestID0_ = _e51;
                } else {
                    float _e52 = bestDistance1_;
                    if (distanceSquared < _e52) {
                        float _e54 = bestDistance1_;
                        bestDistance2_ = _e54;
                        uint _e55 = bestID1_;
                        bestID2_ = _e55;
                        bestDistance1_ = distanceSquared;
                        uint _e56 = candidate;
                        bestID1_ = _e56;
                    } else {
                        bestDistance2_ = distanceSquared;
                        uint _e57 = candidate;
                        bestID2_ = _e57;
                    }
                }
            }
        }
    }
    float _e65 = gu.graph.w;
    uint requestedConnections = metal::min(GRAPH_MAX_CONNECTIONS, naga_f2u32(_e65 + 0.5));
    uint2 loop_bound_3 = uint2(4294967295u);
    bool loop_init_2 = true;
    while(true) {
        if (metal::all(loop_bound_3 == uint2(0u))) { break; }
        loop_bound_3 -= uint2(loop_bound_3.y == 0u, 1u);
        if (!loop_init_2) {
            uint _e150 = slot_1;
            slot_1 = _e150 + 1u;
        }
        loop_init_2 = false;
        uint _e72 = slot_1;
        if (_e72 < GRAPH_MAX_CONNECTIONS) {
        } else {
            break;
        }
        {
            uint _e75 = bestID2_;
            uint _e76 = bestID1_;
            uint _e77 = slot_1;
            uint _e81 = bestID0_;
            uint _e82 = slot_1;
            uint targetID = (_e82 == 0u) ? _e81 : ((_e77 == 1u) ? _e76 : _e75);
            float _e86 = bestDistance2_;
            float _e87 = bestDistance1_;
            uint _e88 = slot_1;
            float _e92 = bestDistance0_;
            uint _e93 = slot_1;
            float distanceSquared_1 = (_e93 == 0u) ? _e92 : ((_e88 == 1u) ? _e87 : _e86);
            uint _e99 = slot_1;
            uint edge = ((id_9 * GRAPH_MAX_CONNECTIONS) + _e99) * 2u;
            uint _e103 = slot_1;
            if ((_e103 < requestedConnections) && (targetID != 4294967295u)) {
                metal::float4 tgt_1 = graphPositions[targetID];
                float proximity = 1.0 - (metal::sqrt(distanceSquared_1) / threshold);
                float _e118 = gu.graph.z;
                float alpha_1 = metal::clamp((_e118 * proximity) * metal::min(source.z, tgt_1.z), 0.0, 1.0);
                graphEdgesMut[edge] = metal::float4(source.xy, alpha_1, 0.0);
                graphEdgesMut[edge + 1u] = metal::float4(tgt_1.xy, alpha_1, 0.0);
            } else {
                graphEdgesMut[edge] = metal::float4(0.0);
                graphEdgesMut[edge + 1u] = metal::float4(0.0);
            }
        }
    }
    return;
}


struct vsLineInput {
};
struct vsLineOutput {
    metal::float4 position [[position]];
    metal::float4 color [[user(loc0), center_perspective]];
};
vertex vsLineOutput vsLine(
  uint vi_1 [[vertex_id]]
, constant Uniforms& u_1 [[buffer(0)]]
, device type_4 const& graphEdgesRo [[buffer(3)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(6)]]
) {
    LineOut out_1 = {};
    metal::float4 edge_1 = graphEdgesRo[vi_1];
    out_1.position = metal::float4(edge_1.xy, 0.0, 1.0);
    metal::float4 _e13 = u_1.appearance;
    float _e18 = u_1.appearance.w;
    out_1.color = metal::float4(_e13.xyz * metal::max(0.05, _e18), edge_1.z);
    LineOut _e24 = out_1;
    const auto _tmp = _e24;
    return vsLineOutput { _tmp.position, _tmp.color };
}


struct fsLineInput {
    metal::float4 color [[user(loc0), center_perspective]];
};
struct fsLineOutput {
    metal::float4 member_6 [[color(0)]];
};
fragment fsLineOutput fsLine(
  fsLineInput varyings_6 [[stage_in]]
, metal::float4 position_1 [[position]]
) {
    const LineOut in_1 = { position_1, varyings_6.color };
    return fsLineOutput { in_1.color };
}

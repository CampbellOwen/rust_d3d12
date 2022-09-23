cbuffer Camera : register(b0) {
    float4x4 M;
    float4x4 P;
}

cbuffer Material : register(b1) {
    uint texture_index;
}

SamplerState s1 : register(s0);

struct PSInput
{
    float4 position : SV_POSITION;
    float4 position_world : POSITION;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD;
};

PSInput VSMain(float3 position : POSITION, float3 normal : NORMAL, float2 uv : TEXCOORD)
{
    PSInput result;

    float4 pos_world = mul(M, float4(position, 1.0));

    float4 pos_clip = mul(P, pos_world);
    result.position = pos_clip;
    result.position_world = pos_world;
    result.normal = normal;
    result.uv = uv;

    return result;
}

float4 PSMain(PSInput input) : SV_TARGET
{
    float3 normal_world = mul(M, float4(input.normal, 0.0)).xyz; // Use 0.0 because normal is a bivector

    float3 l = float3(2.0, 2.0, -1.0) - input.position_world.xyz;
    float l_dist = length(l);
    l = normalize(l);
    float ldotn = clamp(dot(l, normalize(normal_world)), 0.0, 1.0);

    float4 light_col = float4(20.0, 20.0, 20.0, 1.0);

    light_col *= (1 / (l_dist * l_dist));


    Texture2D<float4> tex = ResourceDescriptorHeap[texture_index];

    return tex.Sample(s1, input.uv) * (ldotn * light_col) / 3.14159;
}
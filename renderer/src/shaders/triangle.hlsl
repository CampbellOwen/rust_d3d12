cbuffer Camera : register(b0) {
    float4x4 M;
    float4x4 P;
}

Texture2D t1 : register(t0);
SamplerState s1 : register(s0);
struct PSInput
{
    float4 position : SV_POSITION;
    float4 color : COLOR;
    float2 uv : TEXCOORD;
};

PSInput VSMain(float3 position : POSITION, float3 normal : NORMAL, float2 uv : TEXCOORD)
{
    PSInput result;

    float4 pos_world = mul(M, float4(position, 1.0));

    float3 normal_world = mul(M, float4(normal, 0.0)).xyz; // Use 0.0 because normal is a bivector

    float3 l = float3(2.0, 2.0, -1.0) - pos_world.xyz;
    float l_dist = length(l);
    l = normalize(l);
    float ldotn = clamp(dot(l, normalize(normal_world)), 0.0, 1.0);

    float4 pos_clip = mul(P, pos_world);

    float4 light_col = float4(20.0, 20.0, 20.0, 1.0);

    light_col *= (1 / (l_dist * l_dist));

    result.position = pos_clip;
    result.color = float4(0.9, 0.9, 0.9, 1.0) * (ldotn * light_col) / 3.14159;
    result.uv = uv;
    //result.color = t1.Sample(s1, uv);

    return result;
}

float4 PSMain(PSInput input) : SV_TARGET
{
    return input.color * t1.Sample(s1, input.uv);
}
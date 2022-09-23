cbuffer Camera : register(b0) {
    float4x4 V;
    float4x4 P;
}

cbuffer Material : register(b1) {
    uint texture_index;
}

cbuffer Model : register(b2) {
    float4x4 M;
}


SamplerState s1 : register(s0);

struct PSInput
{
    float4 position : SV_POSITION;
    float4 position_world : POSITION;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD;
};

PSInput VSMain(uint instance : SV_InstanceID, float3 position : POSITION, float3 normal : NORMAL, float2 uv : TEXCOORD)
{
    PSInput result;

    position -= float3(instance*2, 0, instance);

    float4 pos_world = mul(M, float4(position, 1.0));
    float4 pos_view = mul(V, pos_world);

    float4 pos_clip = mul(P, pos_view);
    result.position = pos_clip;
    result.position_world = pos_world;
    result.normal = normalize(mul(V, float4(normal, 0.0)).xyz); // Use 0.0 because normal is a bivector
    result.uv = uv;

    return result;
}

float4 PSMain(PSInput input) : SV_TARGET
{

    float3 l = float3(2.0, 2.0, -1.0) - input.position_world.xyz;
    float l_dist = length(l) / 10.0f;
    l = normalize(l);
    float ldotn = clamp(dot(l, input.normal), 0.0, 1.0);

    float4 light_col = float4(20.0, 20.0, 20.0, 1.0);


    light_col *= (1 / (l_dist ));//* l_dist));


    Texture2D<float4> tex = ResourceDescriptorHeap[texture_index];

    float4 colour = tex.Sample(s1, input.uv) * (float4(0.2,0.2,0.2,1.0) + (ldotn * light_col) / 3.14159); 
    colour = clamp(colour, 0.0, 1.0);

    return colour;
}
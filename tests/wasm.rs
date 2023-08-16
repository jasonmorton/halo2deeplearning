#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[cfg(test)]
mod wasm32 {
    use ark_std::test_rng;
    use ezkl::circuit::modules::elgamal::{ElGamalVariables, ElGamalVariablesSer};
    use ezkl::circuit::modules::poseidon::spec::{PoseidonSpec, POSEIDON_RATE, POSEIDON_WIDTH};
    use ezkl::circuit::modules::poseidon::PoseidonChip;
    use ezkl::circuit::modules::Module;
    use ezkl::graph::modules::POSEIDON_LEN_GRAPH;
    use ezkl::pfsys::{field_to_vecu64, vecu64_to_field, Snark};
    use ezkl::wasm::{
        elgamalDecrypt, elgamalEncrypt, elgamalGenRandom, poseidonHash, prove, verify,
    };
    use halo2curves::bn256::{Fq, Fr, G1Affine};
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    #[cfg(feature = "web")]
    pub use wasm_bindgen_rayon::init_thread_pool;
    use wasm_bindgen_test::*;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    pub const KZG_PARAMS: &[u8] = include_bytes!("../tests/wasm/kzg");
    pub const CIRCUIT_PARAMS: &[u8] = include_bytes!("../tests/wasm/settings.json");
    pub const VK: &[u8] = include_bytes!("../tests/wasm/test.key");
    pub const PK: &[u8] = include_bytes!("../tests/wasm/test.provekey");
    pub const WITNESS: &[u8] = include_bytes!("../tests/wasm/test.witness.json");
    pub const PROOF: &[u8] = include_bytes!("../tests/wasm/test.proof");
    pub const NETWORK: &[u8] = include_bytes!("../tests/wasm/test_network.compiled");

    #[wasm_bindgen_test]
    async fn verify_elgamal_gen_random_wasm() {
        // Generate a seed value
        let seed = [0u8; 32];

        // Convert the seed to a wasm-friendly format
        let wasm_seed = wasm_bindgen::Clamped(seed.to_vec());

        // Use the seed to generate ElGamal variables via WASM function
        let wasm_output = elgamalGenRandom(wasm_seed);

        let wasm_vars: ElGamalVariablesSer = serde_json::from_slice(&wasm_output[..]).unwrap();

        let wasm_vars = ElGamalVariables {
            r: Fr::from_raw(wasm_vars.r),
            pk: G1Affine {
                x: Fq::from_raw(wasm_vars.pk[0]),
                y: Fq::from_raw(wasm_vars.pk[1]),
            },
            sk: Fr::from_raw(wasm_vars.sk),
            window_size: wasm_vars.window_size,
            aux_generator: G1Affine {
                x: Fq::from_raw(wasm_vars.aux_generator[0]),
                y: Fq::from_raw(wasm_vars.aux_generator[1]),
            },
        };

        // Use the same seed to generate ElGamal variables directly
        let mut rng_from_seed = StdRng::from_seed(seed);
        let direct_vars = ElGamalVariables::gen_random(&mut rng_from_seed);

        // Check if both variables are the same
        assert_eq!(direct_vars, wasm_vars)
    }

    #[wasm_bindgen_test]
    async fn verify_elgamal_wasm() {
        let mut rng = test_rng();

        let var = ElGamalVariables::gen_random(&mut rng);

        let mut message: Vec<Fr> = vec![];
        for i in 0..0 {
            message.push(Fr::from(i as u64));
        }

        let pk: [[u64; 4]; 2] = [field_to_vecu64(&var.pk.x), field_to_vecu64(&var.pk.y)];
        let r = field_to_vecu64(&var.r);
        let message_u64: Vec<[u64; 4]> = message
            .clone()
            .into_iter()
            .map(|b| field_to_vecu64(&b))
            .collect();

        let pk = serde_json::to_vec(&pk).unwrap();
        let message_ser = serde_json::to_vec(&message_u64).unwrap();
        let r = serde_json::to_vec(&r).unwrap();

        let cipher = elgamalEncrypt(
            wasm_bindgen::Clamped(pk.clone()),
            wasm_bindgen::Clamped(message_ser.clone()),
            wasm_bindgen::Clamped(r.clone()),
        );

        let sk = field_to_vecu64(&var.sk);
        let sk = serde_json::to_vec(&sk).unwrap();

        let decrypted_message =
            elgamalDecrypt(wasm_bindgen::Clamped(cipher), wasm_bindgen::Clamped(sk));

        let decrypted_message: Vec<[u64; 4]> =
            serde_json::from_slice(&decrypted_message[..]).unwrap();

        let decrypted_message: Vec<Fr> = decrypted_message
            .into_iter()
            .map(|b| vecu64_to_field(&b))
            .collect();

        assert_eq!(message, decrypted_message)
    }

    #[wasm_bindgen_test]
    async fn verify_hash() {
        let mut message: Vec<Fr> = vec![];
        let mut message_vecu64s: Vec<[u64; 4]> = vec![];
        for i in 0..32 {
            message.push(Fr::from(i as u64));
            message_vecu64s.push(field_to_vecu64(&Fr::from(i as u64)));
        }

        let message_ser = serde_json::to_vec(&message_vecu64s).unwrap();

        let hash = poseidonHash(wasm_bindgen::Clamped(message_ser));
        let hash: Vec<Vec<[u64; 4]>> = serde_json::from_slice(&hash[..]).unwrap();

        let hash: Vec<Vec<Fr>> = hash
            .into_iter()
            .map(|v| v.into_iter().map(|b| vecu64_to_field(&b)).collect())
            .collect();

        let reference_hash =
            PoseidonChip::<PoseidonSpec, POSEIDON_WIDTH, POSEIDON_RATE, POSEIDON_LEN_GRAPH>::run(
                message.clone(),
            )
            .unwrap();

        assert_eq!(hash, reference_hash)
    }

    #[wasm_bindgen_test]
    async fn verify_pass() {
        let value = verify(
            wasm_bindgen::Clamped(PROOF.to_vec()),
            wasm_bindgen::Clamped(VK.to_vec()),
            wasm_bindgen::Clamped(CIRCUIT_PARAMS.to_vec()),
            wasm_bindgen::Clamped(KZG_PARAMS.to_vec()),
        );
        assert!(value);
    }

    #[wasm_bindgen_test]
    async fn verify_fail() {
        let og_proof: Snark<Fr, G1Affine> = serde_json::from_slice(&PROOF).unwrap();

        let proof: Snark<Fr, G1Affine> = Snark {
            proof: vec![0; 32],
            protocol: og_proof.protocol,
            instances: vec![vec![Fr::from(0); 32]],
            transcript_type: ezkl::pfsys::TranscriptType::EVM,
        };
        let proof = serde_json::to_string(&proof).unwrap().into_bytes();

        let value = verify(
            wasm_bindgen::Clamped(proof),
            wasm_bindgen::Clamped(VK.to_vec()),
            wasm_bindgen::Clamped(CIRCUIT_PARAMS.to_vec()),
            wasm_bindgen::Clamped(KZG_PARAMS.to_vec()),
        );
        // should fail
        assert!(!value);
    }

    #[wasm_bindgen_test]
    async fn prove_pass() {
        // prove
        let proof = prove(
            wasm_bindgen::Clamped(WITNESS.to_vec()),
            wasm_bindgen::Clamped(PK.to_vec()),
            wasm_bindgen::Clamped(NETWORK.to_vec()),
            wasm_bindgen::Clamped(CIRCUIT_PARAMS.to_vec()),
            wasm_bindgen::Clamped(KZG_PARAMS.to_vec()),
        );
        assert!(proof.len() > 0);

        let value = verify(
            wasm_bindgen::Clamped(proof.to_vec()),
            wasm_bindgen::Clamped(VK.to_vec()),
            wasm_bindgen::Clamped(CIRCUIT_PARAMS.to_vec()),
            wasm_bindgen::Clamped(KZG_PARAMS.to_vec()),
        );
        // should not fail
        assert!(value);
    }
}

/*
An easy-to-use implementation of the Poseidon Hash in the form of a Halo2 Chip. While the Poseidon Hash function
is already implemented in halo2_gadgets, there is no wrapper chip that makes it easy to use in other circuits.
Thanks to https://github.com/summa-dev/summa-solvency/blob/master/src/chips/poseidon/hash.rs for the inspiration (and also helping us understand how to use this).
*/

pub mod rate15_params;
pub mod spec;

// This chip adds a set of advice columns to the gadget Chip to store the inputs of the hash
// compared to `hash_with_instance` this version doesn't use any instance column.
use halo2_gadgets::poseidon::{primitives::*, Hash, Pow5Chip, Pow5Config};
use halo2_proofs::arithmetic::Field;
use halo2_proofs::halo2curves::bn256::Fr as Fp;
use halo2_proofs::{circuit::*, plonk::*};

use std::marker::PhantomData;

use crate::tensor::{Tensor, ValTensor, ValType};

use self::spec::PoseidonSpec;

#[derive(Debug, Clone)]

/// WIDTH, RATE and L are const generics for the struct, which represent the width, rate, and number of inputs for the Poseidon hash function, respectively.
/// This means they are values that are known at compile time and can be used to specialize the implementation of the struct.
/// The actual chip provided by halo2_gadgets is added to the parent Chip.
pub struct PoseidonConfig<const WIDTH: usize, const RATE: usize> {
    ///
    pub hash_inputs: Vec<Column<Advice>>,
    ///
    pub pow5_config: Pow5Config<Fp, WIDTH, RATE>,
}

/// PoseidonChip is a wrapper around the Pow5Chip that adds a set of advice columns to the gadget Chip to store the inputs of the hash
#[derive(Debug, Clone)]
pub struct PoseidonChip<
    S: Spec<Fp, WIDTH, RATE>,
    const WIDTH: usize,
    const RATE: usize,
    const L: usize,
> {
    config: PoseidonConfig<WIDTH, RATE>,
    _marker: PhantomData<S>,
}

impl<S: Spec<Fp, WIDTH, RATE>, const WIDTH: usize, const RATE: usize, const L: usize>
    PoseidonChip<S, WIDTH, RATE, L>
{
    /// Constructs a new PoseidonChip
    pub fn construct(config: PoseidonConfig<WIDTH, RATE>) -> Self {
        Self {
            config,
            _marker: PhantomData,
        }
    }

    /// Configuration of the PoseidonChip
    pub fn configure(
        meta: &mut ConstraintSystem<Fp>,
        hash_inputs: Vec<Column<Advice>>,
    ) -> PoseidonConfig<WIDTH, RATE> {
        let partial_sbox = meta.advice_column();
        let rc_a = (0..WIDTH).map(|_| meta.fixed_column()).collect::<Vec<_>>();
        let rc_b = (0..WIDTH).map(|_| meta.fixed_column()).collect::<Vec<_>>();

        for i in 0..WIDTH {
            meta.enable_equality(hash_inputs[i]);
        }
        meta.enable_constant(rc_b[0]);

        let pow5_config = Pow5Chip::configure::<S>(
            meta,
            hash_inputs.clone().try_into().unwrap(),
            partial_sbox,
            rc_a.try_into().unwrap(),
            rc_b.try_into().unwrap(),
        );

        PoseidonConfig {
            pow5_config,
            hash_inputs,
        }
    }

    /// L is the number of inputs to the hash function
    /// Takes the cells containing the input values of the hash function and return the cell containing the hash output
    /// It uses the pow5_chip to compute the hash
    pub fn hash(
        &self,
        layouter: &mut impl Layouter<Fp>,
        input: ValTensor<Fp>,
        zero_val: AssignedCell<Fp, Fp>,
    ) -> Result<ValTensor<Fp>, Error> {
        // iterate over the input cells in blocks of L
        let mut input_cells: Vec<AssignedCell<Fp, Fp>> = input
            .get_inner_tensor()
            .map_err(|_| Error::Synthesis)?
            .map(|v| match v {
                ValType::PrevAssigned(c) => c,
                _ => panic!("wrong input type, must be previously assigned"),
            })[..]
            .to_vec();

        // do the Tree dance baby
        while input_cells.len() > 1 {
            let mut hashes = vec![];
            for block in input_cells.chunks(L) {
                let mut block = block.to_vec();
                let remainder = block.len() % L;

                if remainder != 0 {
                    block.extend(vec![zero_val.clone(); L - remainder].into_iter());
                }

                let pow5_chip = Pow5Chip::construct(self.config.pow5_config.clone());
                // initialize the hasher
                let hasher = Hash::<_, _, S, ConstantLength<L>, WIDTH, RATE>::init(
                    pow5_chip,
                    layouter.namespace(|| "block_hasher"),
                )?;

                // you may need to 0 pad the inputs so they fit
                let hash = hasher.hash(
                    layouter.namespace(|| "hash"),
                    block.to_vec().try_into().map_err(|_| Error::Synthesis)?,
                );

                hashes.push(hash?);
            }
            input_cells = hashes;
        }

        let result = Tensor::from(input_cells.iter().map(|e| ValType::from(e.clone())));

        Ok(result.into())
    }
}

///
pub fn witness_hash<const L: usize>(
    message: Vec<Fp>,
) -> Result<Value<Fp>, Box<dyn std::error::Error>> {
    let mut hash_inputs = message.clone();
    // do the Tree dance baby
    while hash_inputs.len() > 1 {
        let mut hashes: Vec<Fp> = vec![];
        for block in hash_inputs.chunks(L) {
            let mut block = block.to_vec();
            let remainder = block.len() % L;
            if remainder != 0 {
                block.extend(vec![Fp::ZERO; L - remainder].iter());
            }
            let hash = halo2_gadgets::poseidon::primitives::Hash::<
                _,
                PoseidonSpec,
                ConstantLength<L>,
                15,
                14,
            >::init()
            .hash(block.clone().try_into().unwrap());
            hashes.push(hash);
        }
        hash_inputs = hashes;
    }

    let output = hash_inputs[0];

    Ok(Value::known(output))
}

#[allow(unused)]
mod tests {

    use super::{spec::PoseidonSpec, *};

    use std::marker::PhantomData;

    use halo2_gadgets::poseidon::primitives::Spec;
    use halo2_proofs::{
        circuit::{Layouter, SimpleFloorPlanner, Value},
        plonk::{Circuit, ConstraintSystem},
    };
    use halo2curves::ff::Field;

    const WIDTH: usize = 15;
    const RATE: usize = 14;
    const R: usize = 240;

    struct HashCircuit<S: Spec<Fp, WIDTH, RATE>, const L: usize> {
        message: ValTensor<Fp>,
        // determines whether the message is binary or not / do we need to convert bits to field elements ?
        // For the purpose of this test, witness the result.
        // TODO: Move this into an instance column.
        output: Value<Fp>,
        _spec: PhantomData<S>,
    }

    impl<S: Spec<Fp, WIDTH, RATE>, const L: usize> Circuit<Fp> for HashCircuit<S, L> {
        type Config = PoseidonConfig<WIDTH, RATE>;
        type FloorPlanner = SimpleFloorPlanner;
        type Params = ();

        fn without_witnesses(&self) -> Self {
            let empty_val: Vec<ValType<Fp>> = vec![Value::<Fp>::unknown().into()];
            let message: Tensor<ValType<Fp>> = empty_val.into_iter().into();

            Self {
                message: message.into(),
                output: Value::unknown(),
                _spec: PhantomData,
            }
        }

        fn configure(meta: &mut ConstraintSystem<Fp>) -> PoseidonConfig<WIDTH, RATE> {
            let const_col = meta.fixed_column();
            meta.enable_equality(const_col);
            let hash_inputs = (0..WIDTH).map(|_| meta.advice_column()).collect::<Vec<_>>();
            for input in &hash_inputs {
                meta.enable_equality(input.clone());
            }
            PoseidonChip::<PoseidonSpec, WIDTH, RATE, L>::configure(meta, hash_inputs)
        }

        fn synthesize(
            &self,
            config: PoseidonConfig<WIDTH, RATE>,
            mut layouter: impl Layouter<Fp>,
        ) -> Result<(), Error> {
            let chip: PoseidonChip<PoseidonSpec, WIDTH, RATE, L> =
                PoseidonChip::construct(config.clone());

            let mut hash_inputs = self.message.clone();

            let (message, zero_val) = layouter.assign_region(
                || "load message",
                |mut region| {
                    let message_word = |i: usize| {
                        let value = &self.message.get_inner_tensor().unwrap()[i];
                        let value = match value {
                            ValType::Value(c) => c,
                            _ => panic!("wrong input type, must be previously assigned"),
                        };

                        let x = i % WIDTH;
                        let y = i / WIDTH;

                        region.assign_advice(
                            || format!("load message_{}", i),
                            config.hash_inputs[x],
                            y,
                            || value.clone(),
                        )
                    };

                    let message: Result<Vec<AssignedCell<Fp, Fp>>, Error> =
                        (0..self.message.len()).map(message_word).collect();
                    let message: Tensor<ValType<Fp>> = message?
                        .iter()
                        .map(|x| Into::<ValType<Fp>>::into(x.clone()))
                        .into();

                    let offset = self.message.len() / WIDTH + 1;

                    let zero_val = region
                        .assign_advice_from_constant(|| "", config.hash_inputs[0], offset, Fp::ZERO)
                        .unwrap();

                    Ok((message.into(), zero_val))
                },
            )?;

            let output = &chip
                .hash(&mut layouter, message, zero_val)?
                .get_inner_tensor()
                .unwrap()[0];

            let output = match output {
                ValType::PrevAssigned(v) => v,
                _ => panic!(),
            };

            layouter.assign_region(
                || "constrain output",
                |mut region| {
                    let expected_var = region.assign_advice(
                        || "load output",
                        config.hash_inputs[0],
                        0,
                        || self.output,
                    )?;

                    region.constrain_equal(output.cell(), expected_var.cell())
                },
            )
        }
    }

    #[test]
    fn poseidon_hash() {
        let rng = rand::rngs::OsRng;

        let message = [Fp::random(rng), Fp::random(rng)];
        let output = witness_hash::<2>(message.to_vec()).unwrap();

        let mut message: Tensor<ValType<Fp>> =
            message.into_iter().map(|m| Value::known(m).into()).into();

        let k = 7;
        let circuit = HashCircuit::<PoseidonSpec, 2> {
            message: message.into(),
            output,
            _spec: PhantomData,
        };
        let prover = halo2_proofs::dev::MockProver::run(k, &circuit, vec![]).unwrap();
        assert_eq!(prover.verify(), Ok(()))
    }

    #[test]
    fn poseidon_hash_longer_input() {
        let rng = rand::rngs::OsRng;

        let message = [Fp::random(rng), Fp::random(rng), Fp::random(rng)];
        let output = witness_hash::<3>(message.to_vec()).unwrap();

        let mut message: Tensor<ValType<Fp>> =
            message.into_iter().map(|m| Value::known(m).into()).into();

        let k = 7;
        let circuit = HashCircuit::<PoseidonSpec, 3> {
            message: message.into(),
            output,
            _spec: PhantomData,
        };
        let prover = halo2_proofs::dev::MockProver::run(k, &circuit, vec![]).unwrap();
        assert_eq!(prover.verify(), Ok(()))
    }

    #[test]
    #[ignore]
    fn poseidon_hash_much_longer_input() {
        let rng = rand::rngs::OsRng;

        let mut message: Vec<Fp> = (0..2048).map(|_| Fp::random(rng)).collect::<Vec<_>>();

        let output = witness_hash::<14>(message.clone()).unwrap();

        let mut message: Tensor<ValType<Fp>> =
            message.into_iter().map(|m| Value::known(m).into()).into();

        let k = 17;
        let circuit = HashCircuit::<PoseidonSpec, 14> {
            message: message.into(),
            output,
            _spec: PhantomData,
        };
        let prover = halo2_proofs::dev::MockProver::run(k, &circuit, vec![]).unwrap();
        assert_eq!(prover.verify(), Ok(()))
    }
}

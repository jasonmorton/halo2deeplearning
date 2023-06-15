import ezkl_lib
import os
import pytest
import json

folder_path = os.path.abspath(
    os.path.join(
        os.path.dirname(__file__),
        '.',
    )
)

examples_path = os.path.abspath(
    os.path.join(
        folder_path,
        '..',
        '..',
        'examples',
    )
)

srs_path = os.path.join(folder_path, 'kzg_test.params')
params_k20_path = os.path.join(folder_path, 'kzg_test_k20.params')


def test_table_1l_average():
    """
    Test for table() with 1l_average.onnx
    """
    path = os.path.join(
        examples_path,
        'onnx',
        '1l_average',
        'network.onnx'
    )

    expected_table = (
        " \n"
        "┌─────────┬───────────┬────────┬──────────────┬─────┐\n"
        "│ opkind  │ out_scale │ inputs │ out_dims     │ idx │\n"
        "├─────────┼───────────┼────────┼──────────────┼─────┤\n"
        "│ Input   │ 7         │        │ [1, 3, 2, 2] │ 0   │\n"
        "├─────────┼───────────┼────────┼──────────────┼─────┤\n"
        "│ PAD     │ 7         │ [0]    │ [1, 3, 4, 4] │ 1   │\n"
        "├─────────┼───────────┼────────┼──────────────┼─────┤\n"
        "│ SUMPOOL │ 7         │ [1]    │ [1, 3, 3, 3] │ 2   │\n"
        "├─────────┼───────────┼────────┼──────────────┼─────┤\n"
        "│ RESHAPE │ 7         │ [2]    │ [3, 3, 3]    │ 3   │\n"
        "└─────────┴───────────┴────────┴──────────────┴─────┘"
    )
    assert ezkl_lib.table(path) == expected_table


def test_gen_srs():
    """
    test for gen_srs() with 17 logrows and 20 logrows.
    You may want to comment this test as it takes a long time to run
    """
    ezkl_lib.gen_srs(srs_path, 17)
    assert os.path.isfile(srs_path)

    ezkl_lib.gen_srs(params_k20_path, 20)
    assert os.path.isfile(params_k20_path)


def test_calibrate():
    """
    Test for calibration pass
    """
    data_path = os.path.join(
        examples_path,
        'onnx',
        '1l_average',
        'input.json'
    )
    model_path = os.path.join(
        examples_path,
        'onnx',
        '1l_average',
        'network.onnx'
    )
    output_path = os.path.join(
        folder_path,
        'settings.json'
    )

    # TODO: Dictionary outputs
    res = ezkl_lib.gen_settings(model_path, output_path)
    assert res == True

    res = ezkl_lib.calibrate_settings(
        data_path, model_path, output_path, "resources")
    assert res == True
    assert os.path.isfile(output_path)


def test_forward():
    """
    Test for vanilla forward pass
    """
    data_path = os.path.join(
        examples_path,
        'onnx',
        '1l_average',
        'input.json'
    )
    model_path = os.path.join(
        examples_path,
        'onnx',
        '1l_average',
        'network.onnx'
    )
    output_path = os.path.join(
        folder_path,
        'input_forward.json'
    )
    settings_path = os.path.join(
        folder_path,
        'settings.json'
    )

    res = ezkl_lib.forward(data_path, model_path,
                           output_path, settings_path=settings_path)
    print(res)
    assert res["input_data"] == [[0.05326240137219429, 0.07497049868106842, 0.05235540121793747, 0.02882540039718151, 0.058487001806497574, 0.008225799538195133, 0.0753002017736435, 0.0821458026766777, 0.062279801815748215, 0.024305999279022217, 0.057931698858737946, 0.040442001074552536]]
    assert res["output_data"] == [[0.05322260037064552, 0.12841789424419403, 0.0751952975988388, 0.10546869784593582, 0.20947259664535522, 0.1040038987994194, 0.05224600061774254, 0.08105459809303284, 0.02880850061774254, 0.05859370157122612, 0.06689450144767761, 0.008300700224936008, 0.13378900289535522, 0.22412109375, 0.09033200144767761, 0.0751952975988388, 0.15722650289535522, 0.08203119784593582, 0.0625, 0.08691400289535522, 0.024413999170064926, 0.12060540169477463, 0.18554680049419403, 0.0649413987994194, 0.05810540169477463, 0.0986327975988388, 0.04052729904651642]]
    assert res["input_hashes"] == ['0x0410102678df39df8cc885c02565e1a759336be033c5ce61b7e3ca24c69258ab']
    assert res["output_hashes"] == ['0x27e56f2eb45fc8bc64e5c516afc9ae644dd78a340491b2f5af96badd53284c46']

    with open(output_path, "r") as f:
        data = json.load(f)
    assert data == {"input_data": [[0.053262424, 0.074970566, 0.052355476, 0.028825462, 0.058487028, 0.008225823, 0.07530029, 0.0821458, 0.06227987, 0.024306035, 0.05793174, 0.04044203]], "output_data": [[0.053222656, 0.12841797, 0.07519531, 0.10546875, 0.20947266, 0.104003906, 0.052246094, 0.08105469, 0.028808594, 0.05859375, 0.06689453, 0.008300781, 0.13378906, 0.2241211,
                                                                                                                                                                                                             0.09033203, 0.07519531, 0.15722656, 0.08203125, 0.0625, 0.08691406, 0.024414062, 0.12060547, 0.18554688, 0.064941406, 0.05810547, 0.09863281, 0.040527344]], "input_hashes": [[8270957937025516140, 11801026918842104328, 2203849898884507041, 140307258138425306]], "output_hashes": [[4554067273356176515, 2525802612124249168, 5413776662459769622, 1194961624936436872]]}


def test_mock():
    """
    Test for mock
    """

    data_path = os.path.join(
        folder_path,
        'input_forward.json'
    )

    model_path = os.path.join(
        examples_path,
        'onnx',
        '1l_average',
        'network.onnx'
    )

    settings_path = os.path.join(folder_path, 'settings.json')

    res = ezkl_lib.mock(data_path, model_path,
                        settings_path)
    assert res == True


def test_setup():
    """
    Test for setup
    """

    data_path = os.path.join(
        folder_path,
        'input_forward.json'
    )

    model_path = os.path.join(
        examples_path,
        'onnx',
        '1l_average',
        'network.onnx'
    )

    pk_path = os.path.join(folder_path, 'test.pk')
    vk_path = os.path.join(folder_path, 'test.vk')
    settings_path = os.path.join(folder_path, 'settings.json')

    res = ezkl_lib.setup(
        model_path,
        vk_path,
        pk_path,
        srs_path,
        settings_path,
    )
    assert res == True
    assert os.path.isfile(vk_path)
    assert os.path.isfile(pk_path)
    assert os.path.isfile(settings_path)


def test_setup_evm():
    """
    Test for setup
    """

    data_path = os.path.join(
        folder_path,
        'input_forward.json'
    )

    model_path = os.path.join(
        examples_path,
        'onnx',
        '1l_average',
        'network.onnx'
    )

    pk_path = os.path.join(folder_path, 'test_evm.pk')
    vk_path = os.path.join(folder_path, 'test_evm.vk')
    settings_path = os.path.join(folder_path, 'settings.json')

    res = ezkl_lib.setup(
        model_path,
        vk_path,
        pk_path,
        srs_path,
        settings_path,
    )
    assert res == True
    assert os.path.isfile(vk_path)
    assert os.path.isfile(pk_path)
    assert os.path.isfile(settings_path)


def test_prove_and_verify():
    """
    Test for prove and verify
    """

    data_path = os.path.join(
        folder_path,
        'input_forward.json'
    )

    model_path = os.path.join(
        examples_path,
        'onnx',
        '1l_average',
        'network.onnx'
    )

    pk_path = os.path.join(folder_path, 'test.pk')
    proof_path = os.path.join(folder_path, 'test.pf')
    settings_path = os.path.join(folder_path, 'settings.json')

    res = ezkl_lib.prove(
        data_path,
        model_path,
        pk_path,
        proof_path,
        srs_path,
        "poseidon",
        "single",
        settings_path,
    )
    assert res == True
    assert os.path.isfile(proof_path)

    vk_path = os.path.join(folder_path, 'test.vk')
    res = ezkl_lib.verify(proof_path, settings_path,
                          vk_path, srs_path)
    assert res == True
    assert os.path.isfile(vk_path)


def test_prove_evm():
    """
    Test for prove using evm transcript
    """

    data_path = os.path.join(
        folder_path,
        'input_forward.json'
    )

    model_path = os.path.join(
        examples_path,
        'onnx',
        '1l_average',
        'network.onnx'
    )

    pk_path = os.path.join(folder_path, 'test_evm.pk')
    proof_path = os.path.join(folder_path, 'test_evm.pf')
    settings_path = os.path.join(folder_path, 'settings.json')

    res = ezkl_lib.prove(
        data_path,
        model_path,
        pk_path,
        proof_path,
        srs_path,
        "evm",
        "single",
        settings_path,
    )
    assert res == True
    assert os.path.isfile(proof_path)

    res = ezkl_lib.print_proof_hex(proof_path)
    # to figure out a better way of testing print_proof_hex
    assert type(res) == str


def test_create_evm_verifier():
    """
    Create EVM verifier with solidity code
    In order to run this test you will need to install solc in your environment
    """
    vk_path = os.path.join(folder_path, 'test_evm.vk')
    settings_path = os.path.join(folder_path, 'settings.json')
    deployment_code_path = os.path.join(folder_path, 'deploy.code')
    sol_code_path = os.path.join(folder_path, 'test.sol')

    res = ezkl_lib.create_evm_verifier(
        vk_path,
        srs_path,
        settings_path,
        deployment_code_path,
        sol_code_path
    )

    assert res == True
    assert os.path.isfile(deployment_code_path)
    assert os.path.isfile(sol_code_path)


def test_verify_evm():
    """
    Verifies an evm proof
    In order to run this you will need to install solc in your environment
    """
    proof_path = os.path.join(folder_path, 'test_evm.pf')
    deployment_code_path = os.path.join(folder_path, 'deploy.code')

    # TODO: without optimization there will be out of gas errors
    # sol_code_path = os.path.join(folder_path, 'test.sol')

    res = ezkl_lib.verify_evm(
        proof_path,
        deployment_code_path,
        # sol_code_path
        # optimizer_runs
    )

    assert res == True


def test_aggregate_and_verify_aggr():
    """
    Tests for aggregated proof and verifying aggregate proof
    """
    data_path = os.path.join(
        examples_path,
        'onnx',
        '1l_relu',
        'input.json'
    )

    model_path = os.path.join(
        examples_path,
        'onnx',
        '1l_relu',
        'network.onnx'
    )

    pk_path = os.path.join(folder_path, '1l_relu.pk')
    vk_path = os.path.join(folder_path, '1l_relu.vk')
    settings_path = os.path.join(
        folder_path, '1l_relu_settings.json')

   # TODO: Dictionary outputs
    res = ezkl_lib.gen_settings(model_path, settings_path)
    assert res == True

    res = ezkl_lib.calibrate_settings(
        data_path, model_path, settings_path, "resources")
    assert res == True
    assert os.path.isfile(settings_path)

    ezkl_lib.setup(
        model_path,
        vk_path,
        pk_path,
        srs_path,
        settings_path,
    )

    proof_path = os.path.join(folder_path, '1l_relu.pf')

    ezkl_lib.prove(
        data_path,
        model_path,
        pk_path,
        proof_path,
        srs_path,
        "poseidon",
        "accum",
        settings_path,
    )

    aggregate_proof_path = os.path.join(folder_path, 'aggr_1l_relu.pf')
    aggregate_vk_path = os.path.join(folder_path, 'aggr_1l_relu.vk')

    res = ezkl_lib.aggregate(
        aggregate_proof_path,
        [proof_path],
        [settings_path],
        [vk_path],
        aggregate_vk_path,
        params_k20_path,
        "poseidon",
        20,
        "unsafe"
    )

    assert res == True
    assert os.path.isfile(aggregate_proof_path)
    assert os.path.isfile(aggregate_vk_path)

    res = ezkl_lib.verify_aggr(
        aggregate_proof_path,
        aggregate_vk_path,
        params_k20_path,
        20,
    )
    assert res == True


def test_evm_aggregate_and_verify_aggr():
    """
    Tests for EVM aggregated proof and verifying aggregate proof
    """
    data_path = os.path.join(
        examples_path,
        'onnx',
        '1l_relu',
        'input.json'
    )

    model_path = os.path.join(
        examples_path,
        'onnx',
        '1l_relu',
        'network.onnx'
    )

    pk_path = os.path.join(folder_path, '1l_relu.pk')
    vk_path = os.path.join(folder_path, '1l_relu.vk')
    settings_path = os.path.join(
        folder_path, '1l_relu_settings.json')

    ezkl_lib.gen_settings(
        model_path,
        settings_path,
    )

    ezkl_lib.calibrate_settings(
        data_path,
        model_path,
        settings_path,
        "resources",
    )

    ezkl_lib.setup(
        model_path,
        vk_path,
        pk_path,
        srs_path,
        settings_path,
    )

    proof_path = os.path.join(folder_path, '1l_relu.pf')

    ezkl_lib.prove(
        data_path,
        model_path,
        pk_path,
        proof_path,
        srs_path,
        "poseidon",
        "accum",
        settings_path,
    )

    aggregate_proof_path = os.path.join(folder_path, 'aggr_1l_relu.pf')
    aggregate_vk_path = os.path.join(folder_path, 'aggr_1l_relu.vk')

    res = ezkl_lib.aggregate(
        aggregate_proof_path,
        [proof_path],
        [settings_path],
        [vk_path],
        aggregate_vk_path,
        params_k20_path,
        "evm",
        20,
        "unsafe"
    )

    assert res == True
    assert os.path.isfile(aggregate_proof_path)
    assert os.path.isfile(aggregate_vk_path)

    aggregate_deploy_path = os.path.join(folder_path, 'aggr_1l_relu.code')
    sol_code_path = os.path.join(folder_path, 'aggr_1l_relu.sol')

    res = ezkl_lib.create_evm_verifier_aggr(
        aggregate_vk_path,
        params_k20_path,
        aggregate_deploy_path,
        sol_code_path
    )

    assert res == True
    assert os.path.isfile(aggregate_deploy_path)

    res = ezkl_lib.verify_aggr(
        aggregate_proof_path,
        aggregate_vk_path,
        params_k20_path,
        20,
    )
    assert res == True

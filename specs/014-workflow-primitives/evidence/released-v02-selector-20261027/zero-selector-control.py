import importlib.util,sys
from pathlib import Path
spec=importlib.util.spec_from_file_location('profile',Path.cwd()/'scripts/test-released-v02-profile.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
module.SCENARIOS['negative-selector']=('released_v02_inflight::__nonexistent_case_for_negative_control','tests/integration/workflow-primitives/released_v02_inflight.rs','selector-zero.txt')
sys.argv=['profile','--prepared-run','/tmp/sagat-v03-20261026/released-v02-al_pw2ho','--scenario','negative-selector']
try:
    module.main()
except RuntimeError as error:
    assert 'selected' in str(error),str(error)
    print('zero matching tests rejected')
else:
    raise AssertionError('zero matching tests were falsely accepted as a successful profile')

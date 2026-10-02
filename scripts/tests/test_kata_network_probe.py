import unittest
from scripts.kata_network_probe import parse_network
from scripts.kata_guest_probe import ProbeRefusal

class KataNetworkContract(unittest.TestCase):
    def fixture(self):
        path='/var/run/netns/cnitest-07658767-dee6-864a-bfcc-7542ec81e0d5'
        return {'Network':{'NetworkID':path,'NetworkCreated':True,'Endpoints':[{'Type':'virtual','Veth':{'NetPair':{'NetInterworkingModel':2,'TAPIface':{'Name':'tap0_kata','HardAddr':'02:00:00:00:00:01'},'VirtIface':{'Name':'eth0','HardAddr':'02:00:00:00:00:02'}}}}]},'Config':{'NetworkConfig':{'NetworkID':path,'NetworkCreated':True,'DisableNewNetwork':False,'DanConfigPath':''}}},{'linux':{'namespaces':[{'type':'network','path':''}]}}
    def test_exact_created_veth(self):
        persist,spec=self.fixture();self.assertEqual(parse_network(persist,spec)['endpoints'][0]['tap_name'],'tap0_kata')
    def test_rejects_redirected_or_foreign_layout(self):
        for change in ('path','created','config','oci','physical','model'):
            with self.subTest(change=change):
                persist,spec=self.fixture()
                if change=='path':persist['Network']['NetworkID']='/data/user-root'
                if change=='created':persist['Network']['NetworkCreated']=False
                if change=='config':persist['Config']['NetworkConfig']['NetworkID']+='x'
                if change=='oci':spec['linux']['namespaces'][0]['path']='/proc/1/ns/net'
                if change=='physical':persist['Network']['Endpoints'][0]['Physical']={}
                if change=='model':persist['Network']['Endpoints'][0]['Veth']['NetPair']['NetInterworkingModel']=1
                with self.assertRaises(ProbeRefusal):parse_network(persist,spec)
    def test_empty_netnone_endpoints(self):
        persist,spec=self.fixture();persist['Network']['Endpoints']=None
        self.assertEqual(parse_network(persist,spec)['endpoints'],[])
